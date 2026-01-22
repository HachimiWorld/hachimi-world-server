use crate::db::playlist::{IPlaylistDao, Playlist, PlaylistDao, PlaylistSong};
use crate::db::CrudDao;
use crate::service::playlist::GetDetailError::{CreatorUserNotFound, NotFound, NotOwner};
use crate::service::{song, user};
use crate::web::routes::playlist::{DetailResp, PlaylistItem, SongItem};
use crate::web::state::AppState;
use axum::extract::State;
use chrono::{DateTime, Utc};
use itertools::Itertools;
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;

#[derive(thiserror::Error, Debug)]
pub enum GetDetailError {
    #[error("Playlist {playlist_id} not found")]
    NotFound { playlist_id: i64 },
    #[error("User {user_id:?} is not the owner of playlist {playlist_id}")]
    NotOwner { user_id: Option<i64>, playlist_id: i64 },
    #[error("Creator user of playlist {playlist_id} not found")]
    CreatorUserNotFound { playlist_id: i64 },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error)
}

pub async fn get_detail(state: &State<AppState>, uid: Option<i64>, playlist_id: i64) -> Result<DetailResp, GetDetailError> {
    let playlist = PlaylistDao::get_by_id(&state.sql_pool, playlist_id).await?
        .ok_or_else(|| NotFound { playlist_id })?;

    // Check permission if it's private
    if !playlist.is_public {
        if let Some(uid) = uid && playlist.user_id == uid {
            // Continue
        } else {
            return Err(NotOwner { user_id: uid, playlist_id }.into())
        }
    }

    let playlist_songs = PlaylistDao::list_songs(&state.sql_pool, playlist.id).await?;
    let song_ids = playlist_songs.iter().map(|song| song.song_id).collect_vec();
    let playlist_songs_map: HashMap<i64, PlaylistSong> = playlist_songs.into_iter().map(|x| (x.song_id, x)).collect();

    let songs = song::get_public_detail_with_cache(state.redis_conn.clone(), &state.sql_pool, &song_ids).await?;
    let creator_user = user::get_public_profile(state.redis_conn.clone(), &state.sql_pool, &[playlist.user_id]).await?
        .remove(&playlist.user_id)
        .ok_or_else(|| CreatorUserNotFound { playlist_id })?; // This should never happen

    let mut result = Vec::<SongItem>::new();

    for x in songs {
        if let Some(song) = x {
            if let Some(ps) = playlist_songs_map.get(&song.id) {
                let item = SongItem {
                    song_id: song.id,
                    song_display_id: song.display_id,
                    title: song.title,
                    subtitle: song.subtitle,
                    cover_url: song.cover_url,
                    uploader_name: song.uploader_name,
                    uploader_uid: song.uploader_uid,
                    duration_seconds: song.duration_seconds,
                    order_index: ps.order_index,
                    add_time: ps.add_time,
                };
                result.push(item);
            }
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
        creator_profile: creator_user,
        songs: result,
    };
    Ok(resp)
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistMetadata {
    pub id: i64,
    pub user_id: i64,
    pub user_name: String,
    pub user_avatar_url: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub songs_count: i64,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub async fn list_playlist_metadata(
    redis: ConnectionManager,
    sql_pool: &PgPool,
    playlist_ids: &[i64],
    filter_private: bool
) -> anyhow::Result<HashMap<i64, PlaylistMetadata>> {
    let mut rows = PlaylistDao::list_by_ids(sql_pool, &playlist_ids).await?;
    // Only list public playlists
    if filter_private {
        rows.retain(|x| x.is_public);
    }

    let user_ids = rows.iter().map(|x| x.user_id).collect_vec();
    let counts = PlaylistDao::count_songs(sql_pool, &playlist_ids).await?;
    let playlists: HashMap<i64, Playlist> = rows.into_iter().map(|p| (p.id, p)).collect();
    let users = user::get_public_profile(redis, sql_pool, &user_ids).await?;

    let result: HashMap<i64, _> = playlist_ids
        .into_iter()
        .filter_map(|id| playlists.get(&id))
        .map(|p| PlaylistMetadata {
            id: p.id,
            name: p.name.clone(),
            description: p.description.clone(),
            cover_url: p.cover_url.clone(),
            user_id: p.user_id,
            user_name: users.get(&p.user_id).map_or("Unknown User".to_string(), |u| u.username.clone()),
            create_time: p.create_time,
            update_time: p.update_time,
            songs_count: counts.get(&p.id).cloned().unwrap_or(0),
            user_avatar_url: users.get(&p.user_id).and_then(|u| u.avatar_url.clone()),
        })
        .map(|x| (x.id, x))
        .collect();
    Ok(result)
}