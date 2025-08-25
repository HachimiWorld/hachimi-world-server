use crate::db::song::Song;
use chrono::{DateTime, Utc};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecentSongRedisCache {
    songs: Vec<Song>,
    create_time: DateTime<Utc>
}

pub async fn get_recent_songs(
    redis: &ConnectionManager,
    pool: &PgPool,
) -> anyhow::Result<Vec<Song>> {
    let cache: Option<String> = redis.clone().get("songs:recent").await?;
    match cache {
        Some(cache) => {
            let cache: RecentSongRedisCache = serde_json::from_str(&cache)?;
            return Ok(cache.songs)
        }
        None => {
            let recent_songs = sqlx::query_as!(Song, "SELECT * FROM songs ORDER BY release_time DESC LIMIT 50")
                .fetch_all(pool).await?;
            let value = serde_json::to_string(&RecentSongRedisCache {
                songs: recent_songs.clone(),
                create_time: Utc::now(),
            })?;

            // Cache for 5 minutes
            let _: () = redis.clone().set_ex("songs:recent", value, 300).await?;
            return Ok(recent_songs);
        }
    }

}

pub async fn get_hot_songs(
    redis: &ConnectionManager,
    pool: &PgPool,
) -> anyhow::Result<Vec<Song>> {
    let cache: Option<String> = redis.clone().get("songs:hot").await?;
    match cache {
        Some(cache) => {
            let cache: RecentSongRedisCache = serde_json::from_str(&cache)?;
            return Ok(cache.songs)
        }
        None => {
            let hot_songs = sqlx::query_as!(Song, "SELECT * FROM songs ORDER BY like_count DESC LIMIT 50")
                .fetch_all(pool).await?;
            let value = serde_json::to_string(&RecentSongRedisCache {
                songs: hot_songs.clone(),
                create_time: Utc::now(),
            })?;

            // Cache for 1 hour
            let _: () = redis.clone().set_ex("songs:hot", value, 3600).await?;
            return Ok(hot_songs);
        }
    }
}

/*async fn get_recommend_songs(
    redis: &ConnectionManager,
    pool: &PgPool,
) -> anyhow::Result<Vec<Song>> {
    todo!()
}*/