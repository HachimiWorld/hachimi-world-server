use crate::db::song::{ISongDao, Song, SongDao, SongOriginInfo, SongProductionCrew};
use crate::db::song_tag::{ISongTagDao, SongTagDao};
use crate::db::user::UserDao;
use crate::db::CrudDao;
use crate::service::{song_like};
use crate::web::routes::song::{TagItem};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use rand::Rng;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicSongDetail {
    pub id: i64,
    pub display_id: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub tags: Vec<TagItem>,
    pub duration_seconds: i32,
    pub lyrics: String,
    pub audio_url: String,
    pub cover_url: String,
    pub production_crew: Vec<SongProductionCrew>,
    pub creation_type: i32,
    pub origin_infos: Vec<CreationTypeInfo>,
    pub uploader_uid: i64,
    pub uploader_name: String,
    pub play_count: i64,
    pub like_count: i64,
    pub external_links: Vec<ExternalLink>,
    /// @since 251102
    pub create_time: DateTime<Utc>,
    /// @since 251102
    pub release_time: DateTime<Utc>,
    /// @since 251105
    pub gain: Option<f32>,
    /// @since 251105
    pub explicit: Option<bool>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreationTypeInfo {
    // If `song_id` is Some, the rest fields could be None
    pub song_display_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub url: Option<String>,
    pub origin_type: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalLink {
    pub platform: String,
    pub url: String,
}

impl CreationTypeInfo {
    pub fn from_song_origin_info(x: SongOriginInfo, song_display_id: Option<String>) -> Self {
        CreationTypeInfo {
            song_display_id,
            title: x.origin_title.clone(),
            artist: x.origin_artist.clone(),
            url: x.origin_url.clone(),
            origin_type: x.origin_type,
        }
    }
}

pub async fn get_public_detail_with_cache_by_display_id(
    mut redis: ConnectionManager,
    sql_pool: &PgPool,
    song_display_id: &str,
) -> Result<Option<PublicSongDetail>, anyhow::Error> {
    let cache_key_display_id = format!("song:detail:{}", song_display_id);
    let cache: Option<String> = redis.get(&cache_key_display_id).await?;

    if let Some(cache) = cache {
        if cache == "null" {
            return Ok(None);
        } else if let Ok(v) = serde_json::from_str::<PublicSongDetail>(&cache) {
            return Ok(Some(v));
            // If parse failed, continue to fallback
        }
    }

    let data = get_from_db_by_display_id(&redis, sql_pool, song_display_id).await?;
    match data {
        Some(data) => {
            // Set cache both for id and display_id
            let cache_key = format!("song:detail:{}", data.id);
            let _: () = redis.set_ex(cache_key, serde_json::to_string(&data).unwrap(), 30 * 60).await?;
            let _: () = redis.set_ex(cache_key_display_id, serde_json::to_string(&data).unwrap(), 30 * 60).await?;
            Ok(Some(data))
        }
        None => {
            // Not exists to forbid cache-through
            let _: () = redis.set_ex(cache_key_display_id, "null", 30 * 60).await?;
            Ok(None)
        }
    }
}

pub async fn get_public_detail_with_cache(
    mut redis: ConnectionManager,
    sql_pool: &PgPool,
    song_id: i64,
) -> Result<Option<PublicSongDetail>, anyhow::Error> {
    let cache_key = format!("song:detail:{}", song_id);
    let cache: Option<String> = redis.get(&cache_key).await?;

    if let Some(cache) = cache {
        if cache == "null" {
            return Ok(None);
        } else if let Ok(v) = serde_json::from_str::<PublicSongDetail>(&cache) {
            return Ok(Some(v));
            // If parse failed, continue to fallback
        }
    }

    let data = get_from_db_by_id(&redis, sql_pool, song_id).await?;
    match data {
        Some(data) => {
            // Set cache both for id and display_id
            let cache_key_display_id = format!("song:detail:{}", data.display_id);
            let _: () = redis.set_ex(cache_key, serde_json::to_string(&data).unwrap(), 30 * 60).await?;
            let _: () = redis.set_ex(cache_key_display_id, serde_json::to_string(&data).unwrap(), 30 * 60).await?;
            Ok(Some(data))
        }
        None => {
            let _: () = redis.set_ex(cache_key, "null", 30 * 60).await?;
            Ok(None)
        }
    }
}

async fn get_from_db_by_display_id(
    redis: &ConnectionManager,
    sql_pool: &PgPool,
    song_display_id: &str
) -> anyhow::Result<Option<PublicSongDetail>> {
    // Fallback to database
    let song = if let Some(x) = SongDao::get_by_display_id(sql_pool, song_display_id).await? {
        x
    } else {
        // Song does not exist in the database
        return Ok(None)
    };

    get_from_db(redis, sql_pool, song).await
}

async fn get_from_db_by_id(
    redis: &ConnectionManager,
    sql_pool: &PgPool,
    song_id: i64
) -> anyhow::Result<Option<PublicSongDetail>> {
    // Fallback to database
    let song = if let Some(x) = SongDao::get_by_id(sql_pool, song_id).await? {
        x
    } else {
        // Song does not exist in the database
        return Ok(None)
    };

    get_from_db(redis, sql_pool, song).await
}

async fn get_from_db(
    redis: &ConnectionManager,
    sql_pool: &PgPool,
    song: Song
) -> anyhow::Result<Option<PublicSongDetail>> {
    let tag_ids = SongDao::list_tags_by_song_id(sql_pool, song.id).await?;
    let tags = SongTagDao::list_by_ids(sql_pool, &tag_ids).await?.into_iter().map(|x|
        TagItem {
            id: x.id,
            name: x.name,
            description: x.description,
        }
    ).collect();

    let uploader_name = UserDao::get_by_id(sql_pool, song.uploader_uid).await?
        .map(|x| x.username).unwrap_or_else(|| "Invalid".to_string());

    let origin_infos = SongDao::list_origin_info_by_song_id(sql_pool, song.id).await?;
    let mut id_display_map = HashMap::new();

    for x in &origin_infos {
        match SongDao::get_by_id(sql_pool, x.song_id).await? {
            Some(y) => {
                id_display_map.insert(x.id, y.display_id);
            }
            None => {
                // TODO: Consider to use other way to indicate the song was deleted
                id_display_map.insert(x.id, "deleted".to_string());
            }
        }
    }

    let origin_infos_mapped = origin_infos.into_iter().map(|x| {
        let id = x.origin_song_id;
        CreationTypeInfo::from_song_origin_info(x, id.and_then(|x| id_display_map.get(&x).cloned()))
    }).collect();

    let production_crew = SongDao::list_production_crew_by_song_id(sql_pool, song.id).await?;

    let like_count = song_like::get_song_likes(&redis, sql_pool, song.id).await?;
    let play_count = SongDao::count_plays(sql_pool, song.id).await?;

    let external_links = SongDao::list_external_link_by_song_id(sql_pool, song.id).await?
        .into_iter().map(|x| ExternalLink {
        platform: x.platform,
        url: x.url,
    }).collect();
    let data = PublicSongDetail {
        id: song.id,
        display_id: song.display_id.to_string(),
        title: song.title.to_string(),
        subtitle: song.subtitle.to_string(), // What?
        description: song.description.to_string(),
        tags: tags,
        duration_seconds: song.duration_seconds,
        lyrics: song.lyrics.to_string(),
        audio_url: song.file_url.to_string(),
        cover_url: song.cover_art_url.to_string(),
        production_crew: production_crew,
        creation_type: song.creation_type,
        origin_infos: origin_infos_mapped,
        uploader_uid: song.uploader_uid,
        uploader_name: uploader_name,
        play_count: play_count,
        like_count: like_count,
        external_links: external_links,
        create_time: song.create_time,
        release_time: song.release_time,
        gain: song.gain,
        explicit: song.explicit,
    };

    Ok(Some(data))
}


/// Pattern: JM-AAAA-000
pub fn generate_song_display_id() -> String {
    let mut rng = rand::rng();

    let letters: String = (0..4)
        .map(|_| rng.random_range(b'A'..=b'Z') as char)
        .collect();

    let numbers: String = (0..3)
        .map(|_| rng.random_range(b'0'..=b'9') as char)
        .collect();

    format!("JM-{}-{}", letters, numbers)
}