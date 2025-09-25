use std::time::Duration;
use crate::service::song;
use crate::service::song::PublicSongDetail;
use anyhow::bail;
use chrono::{DateTime, Utc};
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
    redis: &ConnectionManager,
    pool: &PgPool,
) -> anyhow::Result<Vec<PublicSongDetail>> {
    let cache: Option<String> = redis.clone().get("songs:recent_v2").await?;
    match cache {
        Some(cache) => {
            let cache: RecentSongRedisCache = serde_json::from_str(&cache)?;
            warn!("Got recent songs data from cache but could not be parsed");
            Ok(cache.songs)
        }
        None => {
            let guard = lock.lock_with_timeout("lock:songs:recent_v2", Duration::from_secs(10)).await?;

            // Double-check if the cache is available now
            let cache: Option<String> = redis.clone().get("songs:recent_v2").await?;
            if let Some(cache) = cache {
                let cache: RecentSongRedisCache = serde_json::from_str(&cache)?;
                warn!("Got recent songs data from cache but could not be parsed");
                return Ok(cache.songs)
            }

            let recent_song_ids: Vec<i64> = sqlx::query!("SELECT id FROM songs ORDER BY release_time DESC LIMIT 50")
                .fetch_all(pool).await?
                .into_iter().map(|x| x.id).collect();

            let mut songs = Vec::new();

            for x in recent_song_ids {
                match song::get_public_detail_with_cache(redis.clone(), pool, x).await? {
                    Some(data) => {
                        songs.push(data);
                    }
                    None => {
                        // This might happen logically, but will it really happen?
                        bail!("get_recent_songs got none during getting song({x})")
                    }
                };
            }

            let cache = RecentSongRedisCache { songs, create_time: Utc::now() };
            let value = serde_json::to_string(&cache)?;

            // Cache for 5 minutes
            let _: () = redis.clone().set_ex("songs:recent_v2", value, 300).await?;
            drop(guard);
            Ok(cache.songs)
        }
    }
}

pub async fn notify_update(song_id: i64, redis: &ConnectionManager) -> anyhow::Result<()> {
    // Just delete the cache
    // TODO: Use event based notification
    let _: () = redis.clone().del("songs:recent_v2").await?;
    Ok(())
}