use crate::db::playlist::{IPlaylistDao, Playlist, PlaylistDao, PlaylistSong};
use crate::db::song::SongDao;
use crate::db::user::UserDao;
use crate::db::CrudDao;
use crate::util::IsBlank;
use crate::web::jwt::Claims;
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::state::AppState;
use crate::{common, err, ok, service};
use anyhow::{Context};
use async_backtrace::framed;
use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/set_cover", post(set_cover))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .route("/detail_private", get(detail_private))
        .route("/list", get(list))
        .route("/create", post(create))
        .route("/update", post(update))
        .route("/delete", post(delete))
        .route("/add_song", post(add_song))
        .route("/remove_song", post(remove_song))
        .route("/change_order", post(change_order))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailReq {
    pub id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailResp {
    pub playlist_info: PlaylistItem,
    pub songs: Vec<SongItem>,
}

// Basic song information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongItem {
    pub song_id: i64,
    pub song_display_id: String,
    pub title: String,
    pub subtitle: String,
    pub cover_url: String,
    pub uploader_name: String,
    pub uploader_uid: i64,
    pub duration_seconds: i32,
    pub order_index: i32,
    pub add_time: DateTime<Utc>,
}

#[framed]
async fn detail_private(
    claims: Claims,
    state: State<AppState>,
    req: Query<DetailReq>,
) -> WebResult<DetailResp> {
    let playlist = PlaylistDao::get_by_id(&state.sql_pool, req.id).await?
        .ok_or_else(|| common!("not_found", "Playlist not found"))?;

    // Check permission if it's private
    if !playlist.is_public {
        if playlist.user_id != claims.uid() {
            err!("not_owner", "You are not the owner of this playlist")
        }
    }

    let songs = PlaylistDao::list_songs(&state.sql_pool, playlist.id).await?;
    let mut result = Vec::<SongItem>::new();
    for x in songs {
        if let Some(song) = SongDao::get_by_id(&state.sql_pool, x.song_id).await? &&
            let Some(uploader) = UserDao::get_by_id(&state.sql_pool, song.uploader_uid).await?
        {
            let item = SongItem {
                song_id: x.song_id,
                song_display_id: song.display_id.clone(),
                title: song.title.clone(),
                subtitle: song.subtitle.clone(),
                cover_url: song.cover_art_url.clone(),
                uploader_name: uploader.username.clone(),
                uploader_uid: song.uploader_uid,
                duration_seconds: song.duration_seconds,
                order_index: x.order_index,
                add_time: x.add_time,
            };
            result.push(item);
        } else {
            // How to deal with song deleted?
        }
    }

    let resp = DetailResp {
        playlist_info: PlaylistItem {
            id: playlist.id,
            name: playlist.name,
            cover_url: playlist.cover_url,
            description: playlist.description,
            create_time: playlist.create_time,
            is_public: playlist.is_public,
            songs_count: result.len() as i64,
        },
        songs: result,
    };
    ok!(resp)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResp {
    pub playlists: Vec<PlaylistItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub id: i64,
    pub name: String,
    pub cover_url: Option<String>,
    pub description: Option<String>,
    pub create_time: DateTime<Utc>,
    pub is_public: bool,
    pub songs_count: i64,
}

#[framed]
async fn list(
    claims: Claims,
    state: State<AppState>,
) -> WebResult<ListResp> {
    let playlists = PlaylistDao::list_by_user(&state.sql_pool, claims.uid()).await?;

    let mut result = Vec::<PlaylistItem>::new();
    for x in playlists {
        let item = PlaylistItem {
            id: x.id,
            name: x.name,
            cover_url: x.cover_url,
            description: x.description,
            create_time: x.create_time,
            is_public: x.is_public,
            songs_count: PlaylistDao::count_songs(&state.sql_pool, x.id).await?,
        };
        result.push(item);
    }
    ok!(ListResp {
        playlists: result
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlaylistReq {
    pub name: String,
    // pub use_song_cover: bool,
    // pub cover_temp_id: Option<String>,
    pub description: Option<String>,
    pub is_public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlaylistResp {
    pub id: i64,
}

#[framed]
async fn create(
    claims: Claims,
    state: State<AppState>,
    req: Json<CreatePlaylistReq>,
) -> WebResult<CreatePlaylistResp> {
    // Validate input
    if req.name.is_blank() || req.name.chars().count() > 32 {
        err!("invalid_name", "Playlist name invalid")
    }
    if let Some(ref desc) = req.description && desc.chars().count() > 300 {
        err!("description_too_long", "Playlist description is too long")
    }

    let uid = claims.uid();

    // TODO[security](playlist): We don't have lock, so there must be some data racing issues
    let count = PlaylistDao::count_by_user(&state.sql_pool, uid).await?;
    if count > 256 {
        err!("too_many_playlists", "You have too many playlists")
    }

    let entity = Playlist {
        id: 0,
        name: req.name.clone(),
        description: req.description.clone(),
        user_id: uid,
        cover_url: None, // TODO
        is_public: req.is_public,
        create_time: Utc::now(),
        update_time: Utc::now(),
    };
    let id = PlaylistDao::insert(&state.sql_pool, &entity).await?;

    if req.is_public {
        // TODO: Insert to meilisearch
    }

    ok!(CreatePlaylistResp { id })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePlaylistReq {
    pub id: i64,
    pub name: String,
    // pub use_song_cover: bool,
    // pub cover_temp_id: Option<String>,
    pub description: Option<String>,
    pub is_public: bool,
}

#[framed]
async fn update(
    claims: Claims,
    state: State<AppState>,
    req: Json<UpdatePlaylistReq>,
) -> WebResult<()> {
    // Validate input
    if req.name.is_blank() || req.name.chars().count() > 32 {
        err!("invalid_name", "Playlist name invalid")
    }
    if let Some(ref desc) = req.description && desc.chars().count() > 300 {
        err!("description_too_long", "Playlist description is too long")
    }

    let playlist = check_ownership(&claims, &state.sql_pool, req.id).await?;

    PlaylistDao::update_by_id(
        &state.sql_pool,
        &Playlist {
            name: req.name.clone(),
            description: req.description.clone(),
            update_time: Utc::now(),
            ..playlist
        },
    ).await?;
    ok!(())
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePlaylistReq {
    pub id: i64,
}

#[framed]
async fn delete(
    claims: Claims,
    state: State<AppState>,
    req: Json<DeletePlaylistReq>,
) -> WebResult<()> {
    let playlist = check_ownership(&claims, &state.sql_pool, req.id).await?;
    PlaylistDao::delete_by_id(&state.sql_pool, playlist.id).await?;
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddSongReq {
    pub playlist_id: i64,
    pub song_id: i64,
}

#[framed]
async fn add_song(
    claims: Claims,
    state: State<AppState>,
    req: Json<AddSongReq>,
) -> WebResult<()> {
    let playlist = check_ownership(&claims, &state.sql_pool, req.playlist_id).await?;

    let song = SongDao::get_by_id(&state.sql_pool, req.song_id).await?
        .ok_or_else(|| common!("song_not_found", "Song not found"))?;

    let songs = PlaylistDao::list_songs(&state.sql_pool, playlist.id).await?;
    let existed = songs.iter().any(|x| x.song_id == song.id);
    if existed {
        err!("song_existed", "Song {} already exists in the playlist {}", song.id, playlist.id);
    }
    let target_order = songs.len() as i32;
    PlaylistDao::add_song(
        &state.sql_pool,
        &PlaylistSong {
            playlist_id: playlist.id,
            song_id: song.id,
            order_index: target_order,
            add_time: Utc::now(),
        },
    ).await?;

    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveSongReq {
    pub playlist_id: i64,
    pub song_id: i64,
}

#[framed]
async fn remove_song(
    claims: Claims,
    state: State<AppState>,
    req: Json<RemoveSongReq>,
) -> WebResult<()> {
    let playlist = check_ownership(&claims, &state.sql_pool, req.playlist_id).await?;

    PlaylistDao::remove_song(&state.sql_pool, playlist.id, req.song_id).await?;
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeOrderReq {
    pub playlist_id: i64,
    pub song_id: i64,
    /// Start from 0
    pub target_order: usize,
}

#[framed]
async fn change_order(
    claims: Claims,
    state: State<AppState>,
    req: Json<ChangeOrderReq>,
) -> WebResult<()> {
    let playlist = check_ownership(&claims, &state.sql_pool, req.playlist_id).await?;

    let mut songs = PlaylistDao::list_songs(&state.sql_pool, playlist.id).await?;
    songs.sort_by(|a, b| a.order_index.cmp(&b.order_index));

    let src_index = songs.iter().position(|x| x.song_id == req.song_id)
        .ok_or_else(|| common!("song_not_found", "Song not found"))?;

    // Move to target order_index
    if src_index == req.target_order {
        ok!(())
    }
    // Reorder
    if req.target_order > src_index {
        // move down
        songs[src_index..=req.target_order].rotate_left(1);
    } else {
        // move up
        songs[req.target_order..=src_index].rotate_right(1);
    }
    // Apply order
    for (i, song) in songs.iter_mut().enumerate() {
        song.order_index = i as i32;
    }
    
    let mut tx = state.sql_pool.begin().await?;
    PlaylistDao::update_songs_orders(&mut tx, &songs).await?;
    tx.commit().await?;
    ok!(())
}

async fn check_ownership(
    claims: &Claims,
    pool: &PgPool,
    playlist_id: i64,
) -> Result<Playlist, WebError<CommonError>> {
    let playlist = PlaylistDao::get_by_id(pool, playlist_id).await?
        .ok_or_else(|| common!("not_found", "Playlist not found"))?;
    if playlist.user_id != claims.uid() {
        err!("not_owner", "You are not the owner of this playlist")
    }
    Ok(playlist)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetCoverReq {
    pub playlist_id: i64,
}

async fn set_cover(
    claims: Claims,
    state: State<AppState>,
    mut multipart: Multipart,
) -> WebResult<()> {
    let body_field = multipart.next_field().await?
        .with_context(|| "No data field found")?;
    let json = body_field.text().await?;
    let req: SetCoverReq = serde_json::from_str(&json).with_context(|| "Invalid JSON body")?;
    
    let mut playlist = check_ownership(&claims, &state.sql_pool, req.playlist_id).await?;

    let data_field = multipart
        .next_field()
        .await?
        .with_context(|| "No data field found")?;
    let bytes = data_field.bytes().await?;

    // Validate image
    if bytes.len() > 8 * 1024 * 1024 {
        err!("image_too_large", "Image size must be less than 8MB");
    }
    let format_ext = service::upload::validate_image_and_get_ext(bytes.clone()).await?;

    // Upload image
    let sha1 = openssl::sha::sha1(&bytes);
    let filename = format!("images/playlist/{}.{}", hex::encode(sha1), format_ext);
    let result = state.file_host.upload(bytes, &filename).await?;

    playlist.cover_url = Some(result.public_url);
    PlaylistDao::update_by_id(&state.sql_pool, &playlist).await?;
    ok!(())
}