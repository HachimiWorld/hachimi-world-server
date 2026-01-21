use crate::db::playlist::{IPlaylistDao, Playlist, PlaylistDao, PlaylistSong};
use crate::db::song::SongDao;
use crate::db::CrudDao;
use crate::service::playlist::PlaylistMetadata;
use crate::service::upload::ResizeType;
use crate::service::{playlist, song, user};
use crate::util::IsBlank;
use crate::web::jwt::Claims;
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::routes::user::PublicUserProfile;
use crate::web::state::AppState;
use crate::{common, err, ok, search, service};
use anyhow::Context;
use async_backtrace::framed;
use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/set_cover", post(set_cover).layer(DefaultBodyLimit::max(10 * 1024 * 1024)))
        // @since 250121 @experimental
        .route("/detail", get(detail))
        .route("/detail_private", get(detail_private))
        .route("/list", get(list))
        // @since 250121 @experimental
        .route("/list_public_by_user", get(list_public_by_user))
        .route("/list_containing", get(list_containing))
        .route("/create", post(create))
        .route("/update", post(update))
        .route("/delete", post(delete))
        .route("/add_song", post(add_song))
        .route("/remove_song", post(remove_song))
        .route("/change_order", post(change_order))
        // @since 250121 @experimental
        .route("/search", get(search))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailReq {
    pub id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailResp {
    pub playlist_info: PlaylistItem,
    pub songs: Vec<SongItem>,
    /// @since 260121
    pub creator_profile: PublicUserProfile,
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

/// @since 250121
#[framed]
async fn detail(
    claims: Claims,
    state: State<AppState>,
    req: Query<DetailReq>,
) -> WebResult<DetailResp> {
    let resp = playlist::get_detail(&state, Some(claims.uid()), req.id).await?;
    ok!(resp)
}

#[framed]
async fn detail_private(
    claims: Claims,
    state: State<AppState>,
    req: Query<DetailReq>,
) -> WebResult<DetailResp> {
    let resp = playlist::get_detail(&state, Some(claims.uid()), req.id).await?;
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
    let playlist_ids = playlists.iter().map(|x| x.id).collect_vec();
    let count = PlaylistDao::count_songs(&state.sql_pool, &playlist_ids).await?;
    let mut result = Vec::<PlaylistItem>::new();

    for x in playlists {
        let item = PlaylistItem {
            id: x.id,
            name: x.name,
            cover_url: x.cover_url,
            description: x.description,
            create_time: x.create_time,
            is_public: x.is_public,
            songs_count: count.get(&x.id).cloned().unwrap_or(0),
        };
        result.push(item);
    }
    ok!(ListResp {
        playlists: result
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPublicByUserReq {
    pub user_id: i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListPublicByUserResp {
    pub playlists: Vec<PlaylistMetadata>
}

#[framed]
async fn list_public_by_user(
    claims: Claims,
    state: State<AppState>,
    req: Query<ListPublicByUserReq>,
) -> WebResult<ListPublicByUserResp> {
    let playlists = PlaylistDao::list_by_user(&state.sql_pool, req.user_id).await?;
    let public_playlists = playlists.into_iter()
        .filter(|x| x.is_public || x.user_id == claims.uid())
        .collect_vec();

    let playlist_ids = public_playlists.iter().map(|x| x.id).collect_vec();
    let playlists = playlist::list_playlist_metadata(state.redis_conn.clone(), &state.sql_pool, &playlist_ids, false).await?;

    let result = ListPublicByUserResp {
        playlists: playlists.into_iter().map(|(_, v)| v).collect_vec()
    };

    ok!(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListContainingReq {
    pub song_id: i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListContainingResp {
    pub playlist_ids: Vec<i64>
}

async fn list_containing(
    claims: Claims,
    state: State<AppState>,
    req: Query<ListContainingReq>
) -> WebResult<ListContainingResp> {
    let playlists = PlaylistDao::list_containing(&state.sql_pool, req.song_id, claims.uid()).await?;

    let mut result= playlists
        .into_iter()
        .map(|x| x.id)
        .collect_vec();

    ok!(ListContainingResp {playlist_ids: result})
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
        cover_url: None, // TODO: Pick a song cover by default
        is_public: req.is_public,
        create_time: Utc::now(),
        update_time: Utc::now(),
    };
    let id = PlaylistDao::insert(&state.sql_pool, &entity).await?;

    if req.is_public {
        // Write behind, data consistence is not guaranteed.
        search::playlist::add_or_replace_document(
            &state.meilisearch,
            &state.sql_pool,
            &[id],
        ).await?;
    } else {
        // Ensure it won't be searchable.
        let _ = search::playlist::delete_playlist_document(&state.meilisearch, &[id]).await;
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
            is_public: req.is_public,
            update_time: Utc::now(),
            ..playlist
        },
    ).await?;

    // Update search document if needed.
    if req.is_public {
        search::playlist::add_or_replace_document(
            &state.meilisearch,
            &state.sql_pool,
            &[req.id],
        ).await?;
    } else {
        let _ = search::playlist::delete_playlist_document(&state.meilisearch, &[req.id]).await;
    }

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

    let _ = search::playlist::delete_playlist_document(&state.meilisearch, &[playlist.id]).await;

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
    if songs.len() >= 1000 {
        err!("playlist_full", "The playlist is full")
    }
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

    let webp = service::upload::scale_down_to_webp(512, 512, bytes.clone(), ResizeType::Crop, 80f32)
        .map_err(|_| common!("invalid_image", "The image is not supported"))?;

    // Upload image
    let sha1 = openssl::sha::sha1(&webp);
    let filename = format!("images/playlist/{}.webp", hex::encode(sha1));
    let result = state.file_host.upload(Bytes::from(webp), &filename).await?;

    playlist.cover_url = Some(result.public_url);
    playlist.update_time = Utc::now();
    PlaylistDao::update_by_id(&state.sql_pool, &playlist).await?;

    if playlist.is_public {
        search::playlist::add_or_replace_document(
            &state.meilisearch,
            &state.sql_pool,
            &[playlist.id],
        ).await?;
    }

    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchReq {
    pub q: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub sort_by: Option<String>,
    pub user_id: Option<i64>,
}

type SearchPlaylistItem = PlaylistMetadata;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResp {
    pub hits: Vec<SearchPlaylistItem>,
    pub query: String,
    pub processing_time_ms: u64,
    pub total_hits: Option<usize>,
    pub limit: usize,
    pub offset: usize,
}

#[framed]
async fn search(
    state: State<AppState>,
    req: Query<SearchReq>,
) -> WebResult<SearchResp> {
    if req.q.is_blank() {
        err!("invalid_query", "Query must not be blank")
    }

    let sort_method = match req.sort_by.as_deref() {
        Some("relevance") | None => None,
        Some("create_time_desc") => Some(search::playlist::SearchSortMethod::CreateTimeDesc),
        Some("create_time_asc") => Some(search::playlist::SearchSortMethod::CreateTimeAsc),
        Some("update_time_desc") => Some(search::playlist::SearchSortMethod::UpdateTimeDesc),
        Some("update_time_asc") => Some(search::playlist::SearchSortMethod::UpdateTimeAsc),
        Some(other) => err!("invalid_sort_method", "Invalid sort method: {}", other),
    };

    let filter = req.user_id.map(|uid| format!("user_id = {}", uid));

    let search_query = search::playlist::SearchQuery {
        q: req.q.clone(),
        limit: req.limit.map(|x| x.clamp(1, 50)),
        offset: req.offset,
        filter,
        sort_method,
    };

    let result = search::playlist::search_playlists(state.meilisearch.as_ref(), &search_query).await?;
    let hit_ids: Vec<i64> = result.hits.into_iter().map(|x| x.id).collect();

    let hits = playlist::list_playlist_metadata(state.redis_conn.clone(), &state.sql_pool, &hit_ids, false).await?
        .into_iter()
        .map(|(_, v)| v)
        .collect_vec();

    ok!(SearchResp {
        hits,
        query: result.query,
        processing_time_ms: result.processing_time_ms,
        total_hits: result.hits_info.total_hits,
        limit: result.hits_info.limit,
        offset: result.hits_info.offset,
    })
}