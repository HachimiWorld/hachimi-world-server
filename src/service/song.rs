use crate::db::song::{ISongDao, Song, SongDao, SongOriginInfo, SongProductionCrew};
use crate::db::song_tag::{ISongTagDao, SongTagDao};
use crate::db::user::{IUserDao, UserDao};
use crate::db::CrudDao;
use crate::service::{song_like};
use crate::web::routes::song::{TagItem};
use redis::aio::ConnectionManager;
use redis::{AsyncTypedCommands, MSetOptions, SetExpiry};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use chrono::{DateTime, Utc};
use itertools::Itertools;
use rand::Rng;
use tracing::{debug, warn};

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
            let _: () = redis.set_ex(cache_key, serde_json::to_string(&data)?, 30 * 60).await?;
            let _: () = redis.set_ex(cache_key_display_id, serde_json::to_string(&data)?, 30 * 60).await?;
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
    song_id_list: &[i64],
) -> Result<Vec<Option<PublicSongDetail>>, anyhow::Error> {
    if song_id_list.is_empty() { return Ok(Vec::new()) }
    let start = Instant::now();
    let cache_keys = song_id_list.iter().map(|id| format!("song:detail:{}", id))
        .collect::<Vec<_>>();

    let cache: Vec<Option<String>> = redis.mget(&cache_keys).await?;
    let mut cached: HashMap<i64, PublicSongDetail> = HashMap::with_capacity(song_id_list.len());

    let mut missed_ids: Vec<i64> = vec![];
    for (idx, x) in cache.iter().enumerate() {
        let song_id = song_id_list[idx];
        match x {
            Some(cache) => {
                // Cached
                if cache == "null" {

                } else {
                    match serde_json::from_str::<PublicSongDetail>(&cache) {
                        Ok(x) => {
                            cached.insert(song_id, x);
                        }
                        Err(_) => {
                            warn!("Failed to parse cache for song id: {}", song_id);
                        }
                    }
                }
            }
            None => {
                missed_ids.push(song_id);
            }
        }
    }

    // Not cached, fetch from database
    let fetched = get_from_db_by_ids(&redis, sql_pool, &missed_ids).await?
        .into_iter().map(|x| (x.id, x))
        .collect::<HashMap<_, _>>();

    let cache_to_save_items = missed_ids.iter().map(|song_id| {
        let cache_key = format!("song:detail:{}", song_id);
        let item = fetched.get(song_id);
        let cache_items = match item {
            Some(data) => {
                // Set cache both for id and display_id
                let cache_key_display_id = format!("song:detail:{}", data.display_id);
                let v = serde_json::to_string(&data)?;
                vec![
                    (cache_key, v.clone()),
                    (cache_key_display_id, v)
                ]
            }
            None => {
                vec![(cache_key, "null".to_string())]
            }
        };
        Ok::<_, anyhow::Error>(cache_items)
    })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter().flatten().collect::<Vec<_>>();

    if !cache_to_save_items.is_empty() {
        redis.mset_ex(&cache_to_save_items, MSetOptions::default().with_expiration(SetExpiry::EX(rand::random_range(30 * 60..40 * 60)))).await?;
    }

    let result = song_id_list.iter()
        // .map(|x| cached.remove(x).or_else(|| fetched.remove(x)))
        .map(|x| cached.get(x).cloned().or_else(|| fetched.get(x).cloned()))
        .collect_vec();

    debug!("Get public detail with cache spent {} ms", start.elapsed().as_millis());
    Ok(result)
}

pub async fn get_public_detail_with_cache_legacy(
    mut redis: ConnectionManager,
    sql_pool: &PgPool,
    song_id_list: &[i64],
) -> Result<Vec<Option<PublicSongDetail>>, anyhow::Error> {
    let cache_keys = song_id_list.iter().map(|id| format!("song:detail:{}", id))
        .collect::<Vec<_>>();

    let cache: Vec<Option<String>> = redis.mget(&cache_keys).await?;
    let mut result: Vec<Option<PublicSongDetail>> = Vec::with_capacity(song_id_list.len());

    for (idx, x) in cache.iter().enumerate() {
        let song_id = song_id_list[idx];
        match x {
            Some(cache) => {
                // Cached
                if cache == "null" {
                    result.push(None);
                } else {
                    match serde_json::from_str::<PublicSongDetail>(&cache) {
                        Ok(x) => {
                            result.push(Some(x))
                        }
                        Err(_) => {
                            result.push(None);
                            warn!("Failed to parse cache for song id: {}", song_id);
                        }
                    }
                }
            }
            None => {
                // Not cached, fetch from database
                // TODO: Batch fetch from database
                let data = get_from_db_by_id(&redis, sql_pool, song_id).await?;
                let cache_key = format!("song:detail:{}", song_id);
                match data {
                    Some(data) => {
                        // Set cache both for id and display_id
                        let cache_key_display_id = format!("song:detail:{}", data.display_id);
                        let _: () = redis.set_ex(cache_key, serde_json::to_string(&data)?, rand::random_range(30 * 60..40 * 60)).await?;
                        let _: () = redis.set_ex(cache_key_display_id, serde_json::to_string(&data)?, rand::random_range(30 * 60..40 * 60)).await?;
                        result.push(Some(data))
                    }
                    None => {
                        let _: () = redis.set_ex(cache_key, "null", rand::random_range(30 * 60..40 * 60)).await?;
                        result.push(None)
                    }
                }
            }
        }
    }

    Ok(result)
}

async fn get_from_db_by_display_id(
    redis: &ConnectionManager,
    sql_pool: &PgPool,
    song_display_id: &str,
) -> anyhow::Result<Option<PublicSongDetail>> {
    // Fallback to database
    let song = if let Some(x) = SongDao::get_by_display_id(sql_pool, song_display_id).await? {
        x
    } else {
        // Song does not exist in the database
        return Ok(None)
    };

    compose_from_db(redis, sql_pool, song).await
}

async fn get_from_db_by_id(
    redis: &ConnectionManager,
    sql_pool: &PgPool,
    song_id: i64,
) -> anyhow::Result<Option<PublicSongDetail>> {
    let song = if let Some(x) = SongDao::get_by_id(sql_pool, song_id).await? {
        x
    } else {
        // Song does not exist in the database
        return Ok(None)
    };
    compose_from_db(redis, sql_pool, song).await
}

async fn get_from_db_by_ids(
    redis: &ConnectionManager,
    sql_pool: &PgPool,
    song_ids: &[i64],
) -> anyhow::Result<Vec<PublicSongDetail>> {
    let songs = SongDao::list_by_ids(sql_pool, song_ids).await?;

    compose_from_db_batch(redis, sql_pool, &songs).await
}

async fn compose_from_db_batch(
    redis: &ConnectionManager,
    sql_pool: &PgPool,
    songs: &[Song],
) -> anyhow::Result<Vec<PublicSongDetail>> {
    // WTF did I write?
    let song_ids = Arc::new(songs.iter().map(|x| x.id).collect_vec());

    let a = tokio::spawn({
        let sql_pool = sql_pool.clone();
        let song_ids = Arc::clone(&song_ids);
        async move {
            let tag_id_map = SongTagDao::list_by_song_ids(&sql_pool, &song_ids).await?;
            let tag_ids = tag_id_map.values().flatten().cloned().collect_vec();
            let tags_ref = SongTagDao::list_by_ids(&sql_pool, &tag_ids).await?
                .into_iter().map(|x| (x.id, x))
                .collect::<HashMap<_, _>>();
            Ok::<_, anyhow::Error>((tag_id_map, tag_ids, tags_ref))
        }
    });

    let user_ids = songs.iter().map(|x| x.uploader_uid).collect_vec();
    let b = tokio::spawn({
        let sql_pool = sql_pool.clone();
        async move {
            let uploader_users_map = UserDao::list_by_ids(&sql_pool, &user_ids).await?
                .into_iter().map(|x| (x.id, x))
                .collect::<HashMap<_, _>>();
            Ok::<_, anyhow::Error>(uploader_users_map)
        }
    });
    let c = tokio::spawn({
        let sql_pool = sql_pool.clone();
        let song_ids = Arc::clone(&song_ids);
        async move {
            let origin_infos = SongDao::list_origin_info_by_song_ids(&sql_pool, &song_ids).await?;
            let origin_info_ref = SongDao::list_by_ids(
                &sql_pool,
                &origin_infos.iter().filter_map(|x| x.origin_song_id).collect::<Vec<_>>(),
            ).await?
                .into_iter().map(|x| (x.id, x))
                .collect::<HashMap<_, _>>();

            let origin_info_map = origin_infos
                .into_iter().into_group_map_by(|x| x.song_id);
            Ok::<_, anyhow::Error>((origin_info_ref, origin_info_map))
        }
    });

    let d = tokio::spawn({
        let sql_pool = sql_pool.clone();
        let song_ids = Arc::clone(&song_ids);
        async move {
            let external_links_map = SongDao::list_external_link_by_song_ids(&sql_pool, &song_ids).await?
                .into_iter().into_group_map_by(|x| x.song_id);
            Ok::<_, anyhow::Error>(external_links_map)
        }
    });

    let play_counts_map_fut = SongDao::count_plays_batch(sql_pool, &song_ids);

    let f = tokio::spawn({
        let sql_pool = sql_pool.clone();
        let song_ids = Arc::clone(&song_ids);
        async move {
            let production_crew = SongDao::list_production_crew_by_song_ids(&sql_pool, &song_ids).await?
                .into_iter().into_group_map_by(|x| x.song_id);
            Ok::<_, anyhow::Error>(production_crew)
        }
    });

    // let like_count = song_like::get_song_likes(&redis, sql_pool, song.id).await?;

    let (a, b, c, d, e, f) = tokio::join!(a, b, c, d, play_counts_map_fut, f);
    let (
        (mut tag_id_map, _tag_ids, tags_ref),
        uploader_users_map,
        (origin_info_ref, mut origin_info_map),
        mut external_links_map,
        mut play_counts_map,
        mut production_crew,
    ) = (a??, b??, c??, d??, e?, f??);


    let result = songs.into_iter().map(|song| {
        let tag_ids = tag_id_map.remove(&song.id).unwrap_or_else(|| vec![]);
        let tags = tag_ids.into_iter().filter_map(|tag_id| {
            tags_ref.get(&tag_id).cloned()
        }).map(|x| TagItem {
            id: x.id,
            name: x.name,
            description: x.description,
        }).collect_vec();

        let production_crew = production_crew.remove(&song.id).unwrap_or_else(|| vec![]);
        let origin_infos = origin_info_map.remove(&song.id).unwrap_or_else(|| vec![])
            .into_iter().map(|info| {
            let id = info.origin_song_id;
            let display_id = id.and_then(|x| origin_info_ref.get(&x).map(|x| x.display_id.clone()));
            CreationTypeInfo::from_song_origin_info(info, display_id)
        }).collect_vec();
        let play_count = play_counts_map.remove(&song.id).unwrap_or(0);
        let like_count = 0;
        let external_links = external_links_map.remove(&song.id).unwrap_or_else(|| vec![])
            .into_iter().map(|x| ExternalLink { platform: x.platform, url: x.url })
            .collect_vec();
        let uploader_name = uploader_users_map.get(&song.uploader_uid)
            .map(|x| x.username.clone())
            .unwrap_or("unknown".to_string()).clone();

        let data = PublicSongDetail {
            id: song.id,
            display_id: song.display_id.clone(),
            title: song.title.clone(),
            subtitle: song.subtitle.clone(),
            description: song.description.clone(),
            tags,
            duration_seconds: song.duration_seconds,
            lyrics: song.lyrics.clone(),
            audio_url: song.file_url.clone(),
            cover_url: song.cover_art_url.clone(),
            production_crew,
            creation_type: song.creation_type,
            origin_infos,
            uploader_uid: song.uploader_uid,
            uploader_name,
            play_count,
            like_count,
            external_links,
            create_time: song.create_time,
            release_time: song.release_time,
            gain: song.gain,
            explicit: song.explicit,
        };
        data
    }).collect_vec();

    Ok(result)
}

async fn compose_from_db(
    redis: &ConnectionManager,
    sql_pool: &PgPool,
    song: Song,
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