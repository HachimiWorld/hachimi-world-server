use std::time::{Duration, Instant};
use crate::service::song;
use crate::service::song::PublicSongDetail;
use anyhow::bail;
use chrono::{DateTime, Utc};
use metrics::histogram;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::warn;
use crate::util::redlock::RedLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentSongRedisCache {
    pub songs: Vec<PublicSongDetail>,
    pub create_time: DateTime<Utc>,
}

pub async fn get_recent_songs(
    lock: RedLock,
    redis: ConnectionManager,
    pool: &PgPool,
) -> anyhow::Result<Vec<PublicSongDetail>> {
    let cache = get_from_cache(redis.clone()).await?;

    match cache {
        Some(cache) => {
            Ok(cache)
        }
        None => {
            let guard = lock.lock_with_timeout("lock:songs:recent_v2", Duration::from_secs(10)).await?;

            // Double-check if the cache is available now
            // TODO: Rewrite this use redis based RwLock? Or we can just use memory RwLock for single instance because the service instances wont be too many.
            let cache = get_from_cache(redis.clone()).await?;
            if let Some(cache) = cache {
                return Ok(cache);
            }

            // Or get it from the database
            let songs = get_from_db(redis.clone(), pool).await?;
            save_cache(redis, &songs).await?;
            drop(guard);
            Ok(songs)
        }
    }
}

async fn get_from_cache(redis: ConnectionManager) -> anyhow::Result<Option<Vec<PublicSongDetail>>> {
    let cache: Option<String> = redis.clone().get("songs:recent_v2").await?;
    match cache {
        Some(cache) => {
            match serde_json::from_str::<RecentSongRedisCache>(&cache) {
                Ok(x) => {
                    Ok(Some(x.songs))
                }
                Err(e) => {
                    warn!("Got recent songs data from cache but could not be parsed: {e:?}");
                    Ok(None)
                }
            }
        }
        None => {
            Ok(None)
        }
    }
}

async fn save_cache(mut redis: ConnectionManager, songs: &[PublicSongDetail]) -> anyhow::Result<()> {
    let cache = RecentSongRedisCache { songs: songs.to_vec(), create_time: Utc::now() };
    let value = serde_json::to_string(&cache)?;

    // Cache for 5 minutes
    let _: () = redis.set_ex("songs:recent_v2", value, 300).await?;
    Ok(())
}

async fn get_from_db(mut redis: ConnectionManager, pool: &PgPool) -> anyhow::Result<Vec<PublicSongDetail>> {
    let start = Instant::now();
    let recent_song_ids: Vec<i64> = sqlx::query!("SELECT id FROM songs ORDER BY release_time DESC LIMIT 300")
        .fetch_all(pool).await?
        .into_iter().map(|x| x.id).collect();

    let mut songs = Vec::new();

    for x in recent_song_ids {
        match song::get_public_detail_with_cache(redis.clone(), pool, x).await? {
            Some(mut data) => {
                // TODO: Lyrics is unnecessary for recomment result, temporarily set to empty to save network usage.
                data.lyrics.clear();
                songs.push(data);
            }
            None => {
                // This might happen logically, but will it really happen?
                bail!("get_recent_songs got none during getting song({x})")
            }
        };
    }
    histogram!("recommend_get_from_db_duration_seconds").record(start.elapsed().as_secs_f64());
    Ok(songs)
}

pub async fn notify_update(song_id: i64, mut redis: ConnectionManager) -> anyhow::Result<()> {
    // Just delete the cache
    // TODO: Use event based notification
    let _: () = redis.del("songs:recent_v2").await?;
    Ok(())
}