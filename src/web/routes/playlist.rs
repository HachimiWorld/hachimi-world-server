use crate::web::result::{CommonError, WebResponse};
use crate::web::result::WebError;
use axum::{Json, RequestExt, Router};
use axum::extract::{Query, Request, State};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::db::CrudDao;
use crate::db::playlist::{IPlaylistDao, Playlist, PlaylistDao, PlaylistSong};
use crate::{err, ok};
use crate::db::song::SongDao;
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/detail_private", get(detail_private))
        .route("/list", get(list))
        .route("/create", post(create))
        .route("/update", post(update))
        .route("/delete", post(delete))
        .route("/add_song", post(add_song))
        .route("/remove_song", post(remove_song))
        .route("/change_order", post(add_song))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailReq {
    pub id: i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailResp {
    pub playlist_info: PlaylistItem,
    pub songs: Vec<PlaylistSong>
}

async fn detail_private(
    claims: Claims,
    state: State<AppState>,
    req: Query<DetailReq>,
    raw_req: Request,
) -> WebResult<DetailResp> {
    let playlist_dao = PlaylistDao::new(state.sql_pool.clone());
    let playlist = playlist_dao.get_by_id(req.id).await?
        .ok_or_else(|| WebError::common("not_found", "Playlist not found"))?;
    
    // Check permission if it's private
    if !playlist.is_public {
        if playlist.user_id != claims.uid() {
            err!("not_owner", "You are not the owner of this playlist")
        }
        
        /*match claims {
            Some(v) => {
                if playlist.user_id != v.uid() {
                    err!("not_owner", "You are not the owner of this playlist")
                }
            }
            None => err!("not_owner", "You are not the owner of this playlist")
        }*/
    }

    let songs = playlist_dao.list_songs(
        playlist.id,
    ).await?;

    let resp = DetailResp {
        playlist_info: PlaylistItem {
            name: playlist.name,
            cover_url: playlist.cover_url,
            description: playlist.description,
            create_time: Utc::now(),
            is_public: playlist.is_public,
            songs_count: songs.len() as i64,
        },
        songs,
    };
    ok!(resp)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResp {
    pub playlists: Vec<PlaylistItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub name: String,
    pub cover_url: Option<String>,
    pub description: Option<String>,
    pub create_time: DateTime<Utc>,
    pub is_public: bool,
    pub songs_count: i64,
}

async fn list(
    claims: Claims,
    state: State<AppState>
) -> WebResult<ListResp> {
    let playlist_dao = PlaylistDao::new(state.sql_pool.clone());
    let playlists = playlist_dao.list_by_user(claims.uid()).await?;

    let mut result = Vec::<PlaylistItem>::new();
    for x in playlists {
        let item = PlaylistItem {
            name: x.name,
            cover_url: x.cover_url,
            description: x.description,
            create_time: x.create_time,
            is_public: x.is_public,
            songs_count: playlist_dao.count_songs(claims.uid()).await?
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
    pub is_public: bool
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlaylistResp {
    pub id: i64
}

async fn create(
    claims: Claims,
    state: State<AppState>,
    req: Json<CreatePlaylistReq>
) -> WebResult<CreatePlaylistResp> {
    // Validate input
    if req.name.chars().count() > 32 {
        err!("name_too_long", "Playlist name is too long")
    }
    if let Some(ref desc) = req.description && desc.chars().count() > 300 {
        err!("description_too_long", "Playlist description is too long")
    }

    let uid = claims.uid();
    let playlist_dao = PlaylistDao::new(state.sql_pool.clone());

    let count = playlist_dao.count_by_user(uid).await?;
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
    let id = playlist_dao.insert(&entity).await?;

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
    pub is_public: bool
}

async fn update(
    claims: Claims,
    state: State<AppState>,
    req: Json<UpdatePlaylistReq>
) -> WebResult<()> {
    // Validate input
    if req.name.chars().count() > 32 {
        err!("name_too_long", "Playlist name is too long")
    }
    if let Some(ref desc) = req.description && desc.chars().count() > 300 {
        err!("description_too_long", "Playlist description is too long")
    }

    let playlist_dao = PlaylistDao::new(state.sql_pool.clone());
    let playlist = check_ownership(&claims, &playlist_dao, req.id).await?;

    playlist_dao.update_by_id(
        &Playlist {
            name: req.name.clone(),
            description: req.description.clone(),
            update_time: Utc::now(),
            ..playlist
        }
    ).await?;
    ok!(())
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePlaylistReq {
    pub id: i64
}

async fn delete(
    claims: Claims,
    state: State<AppState>,
    req: Json<DeletePlaylistReq>
) -> WebResult<()> {
    let playlist_dao = PlaylistDao::new(state.sql_pool.clone());
    let playlist = check_ownership(&claims, &playlist_dao, req.id).await?;
    playlist_dao.delete_by_id(playlist.id).await?;
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddSongReq {
    pub playlist_id: i64,
    pub song_id: i64
}

async fn add_song(
    claims: Claims,
    state: State<AppState>,
    req: Json<AddSongReq>
) -> WebResult<()> {
    let playlist_dao = PlaylistDao::new(state.sql_pool.clone());
    let playlist = check_ownership(&claims, &playlist_dao, req.playlist_id).await?;

    let song_dao = SongDao::new(state.sql_pool.clone());
    let song = song_dao.get_by_id(playlist.id).await?
        .ok_or_else(|| WebError::common("song_not_found", "Song not found"))?;

    let songs = playlist_dao.list_songs(playlist.id).await?;
    let target_order = songs.len() as i32;
    playlist_dao.add_song(
        &PlaylistSong {
            playlist_id: playlist.id,
            song_id: song.id,
            order_index: target_order,
            add_time: Utc::now(),
        }
    ).await?;

    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveSongReq {
    pub playlist_id: i64,
    pub song_id: i64
}

async fn remove_song(
    claims: Claims,
    state: State<AppState>,
    req: Json<RemoveSongReq>
) -> WebResult<()> {
    let playlist_dao = PlaylistDao::new(state.sql_pool.clone());
    let playlist = check_ownership(&claims, &playlist_dao, req.playlist_id).await?;

    playlist_dao.remove_song(playlist.id, req.song_id).await?;
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeOrder {
    pub playlist_id: i64,
    pub song_id: i64,
    pub target_order: i64
}

async fn change_order(state: AppState) -> WebResult<()> {
    todo!()
}

async fn check_ownership(
    claims: &Claims,
    playlist_dao: &PlaylistDao,
    playlist_id: i64
) -> Result<Playlist, WebError<CommonError>> {
    let playlist = playlist_dao.get_by_id(playlist_id).await?
        .ok_or_else(|| WebError::common("not_found", "Playlist not found"))?;
    if playlist.user_id != claims.uid() {
        err!("not_owner", "You are not the owner of this playlist")
    }
    Ok(playlist)
}