use std::ops::Sub;
use std::time::{Duration, Instant};
use crate::service::song;
use crate::service::song::{get_public_detail_with_cache, PublicSongDetail};
use anyhow::bail;
use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use futures::{TryStreamExt};
use metrics::histogram;
use rand::prelude::SliceRandom;
use redis::aio::ConnectionManager;
use redis::{AsyncIter, AsyncTypedCommands};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Pool, Postgres};
use tracing::warn;
use crate::db::song::{ISongDao, SongDao};
use crate::util;
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
    cursor: Option<DateTime<Utc>>, limit: i32, after: bool,
) -> anyhow::Result<Vec<PublicSongDetail>> {
    let cache = get_from_cache(redis.clone(), cursor, limit, after).await?;

    match cache {
        Some(cache) => {
            Ok(cache)
        }
        None => {
            let guard = lock.lock_with_timeout("lock:songs:recent_v2", Duration::from_secs(10)).await?;

            // Double-check if the cache is available now
            // TODO: Rewrite this use redis based RwLock? Or we can just use memory RwLock for single instance because the service instances wont be too many.
            let cache = get_from_cache(redis.clone(), cursor, limit, after).await?;
            if let Some(cache) = cache {
                return Ok(cache);
            }

            // Or get it from the database
            let songs = get_from_db(redis.clone(), pool, cursor, limit, after).await?;
            save_cache(redis, &songs, cursor, limit, after).await?;
            drop(guard);
            Ok(songs)
        }
    }
}

async fn get_from_cache(redis: ConnectionManager, cursor: Option<DateTime<Utc>>, limit: i32, after: bool) -> anyhow::Result<Option<Vec<PublicSongDetail>>> {
    let cache: Option<String> = redis.clone().get(build_recent_redis_key(cursor, limit, after)).await?;
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

async fn save_cache(mut redis: ConnectionManager, songs: &[PublicSongDetail], cursor: Option<DateTime<Utc>>, limit: i32, after: bool) -> anyhow::Result<()> {
    let cache = RecentSongRedisCache { songs: songs.to_vec(), create_time: Utc::now() };
    let value = serde_json::to_string(&cache)?;

    // Cache for 5 minutes
    let _: () = redis.set_ex(build_recent_redis_key(cursor, limit, after), value, 300).await?;
    Ok(())
}

fn build_recent_redis_key(cursor: Option<DateTime<Utc>>, limit: i32, after: bool) -> String {
    format!(
        "songs:recent_v2:{cursor}:{limit}:{after}",
        cursor = cursor.map(|x| x.date_naive().to_string()
    ).unwrap_or("latest".to_string()))
}

async fn get_from_db(mut redis: ConnectionManager, pool: &PgPool, cursor: Option<DateTime<Utc>>, limit: i32, after: bool) -> anyhow::Result<Vec<PublicSongDetail>> {
    let cursor = cursor.unwrap_or_else(|| Utc::now());
    let start = Instant::now();
    let recent_songs: Vec<_> = if after {
        SongDao::list_by_create_time_after(pool, cursor, limit as i64).await?
    } else {
        SongDao::list_by_create_time_before(pool, cursor, limit as i64).await?
    };

    let mut songs = Vec::new();

    // Such a waste...
    for x in recent_songs {
        match song::get_public_detail_with_cache(redis.clone(), pool, x.id).await? {
            Some(mut data) => {
                // TODO: Lyrics is unnecessary for recomment result, temporarily set to empty to save network usage.
                data.lyrics.clear();
                songs.push(data);
            }
            None => {
                // This might happen logically, but will it really happen?
                bail!("get_recent_songs got none during getting song({})", x.id)
            }
        };
    }
    histogram!("recommend_get_from_db_duration_seconds").record(start.elapsed().as_secs_f64());
    Ok(songs)
}

pub async fn notify_update(song_id: i64, mut redis: ConnectionManager) -> anyhow::Result<()> {
    // Just delete the cache
    // TODO: Use event based notification
    let keys: AsyncIter<String> = redis.scan_match("songs:recent_v2:latest:*").await?;
    let keys = keys.try_collect::<Vec<_>>().await?;

    redis.del(&keys).await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendRedisCache {
    pub songs: Vec<PublicSongDetail>,
    pub create_time: DateTime<Utc>,
}

pub async fn get_recommend_anonymous(
    ip: &str,
    lock: RedLock,
    redis: ConnectionManager,
    pool: &PgPool,
) -> anyhow::Result<Vec<PublicSongDetail>> {
    let anonymous_uid = util::convert_ip_to_anonymous_uid(&ip)?;
    // 32 groups
    let hash = anonymous_uid % 32 + 1;

    get_recommend(-hash, lock, redis, pool).await
}

/// Return random 30 songs for a user in one day
pub async fn get_recommend(
    user_id: i64,
    lock: RedLock,
    redis: ConnectionManager,
    pool: &PgPool,
) -> anyhow::Result<Vec<PublicSongDetail>> {
    // Refresh at 06:00+8
    let date = Utc::now().with_timezone(&chrono_tz::Asia::Shanghai).sub(TimeDelta::hours(6)).date_naive();
    let cache = get_from_cache_recommend(redis.clone(), user_id, &date).await?;
    match cache {
        Some(cache) => Ok(cache),
        None => {
            let guard = lock.lock_with_timeout(
                &format!("lock:songs:recommend:{}", user_id),
                Duration::from_secs(10),
            ).await?;

            let cache = get_from_cache_recommend(redis.clone(), user_id, &date).await?;
            if let Some(cache) = cache {
                return Ok(cache);
            }

            let mut songs = get_from_db_recommend(redis.clone(), pool).await?;
            songs.shuffle(&mut rand::rng());

            save_cache_recommend(redis, user_id, &songs, &date).await?;
            drop(guard);
            Ok(songs)
        }
    }
}

async fn get_from_cache_recommend(
    mut redis: ConnectionManager,
    user_id: i64,
    date: &NaiveDate,
) -> anyhow::Result<Option<Vec<PublicSongDetail>>> {
    let cache: Option<String> = redis.get(format!("songs:recommend:{}:{}", user_id, date)).await?;
    match cache {
        Some(cache) => match serde_json::from_str::<RecommendRedisCache>(&cache) {
            Ok(x) => Ok(Some(x.songs)),
            Err(e) => {
                warn!("Got recommend songs data from cache but could not be parsed: {e:?}");
                Ok(None)
            }
        },
        None => Ok(None),
    }
}

async fn save_cache_recommend(
    mut redis: ConnectionManager,
    user_id: i64,
    songs: &[PublicSongDetail],
    date: &NaiveDate,
) -> anyhow::Result<()> {
    let cache = RecommendRedisCache {
        songs: songs.to_vec(),
        create_time: Utc::now(),
    };
    let value = serde_json::to_string(&cache)?;

    // Cache for 1 day
    let _: () = redis
        .set_ex(format!("songs:recommend:{}:{}", user_id, date.to_string()), value, 86400)
        .await?;
    Ok(())
}

async fn get_from_db_recommend(
    redis: ConnectionManager,
    pool: &PgPool,
) -> anyhow::Result<Vec<PublicSongDetail>> {
    let start = Instant::now();
    let random_song_ids: Vec<i64> = sqlx::query!("SELECT id FROM songs TABLESAMPLE SYSTEM_ROWS(30)")
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|x| x.id)
        .collect();

    let mut songs = Vec::new();

    for x in random_song_ids {
        match song::get_public_detail_with_cache(redis.clone(), pool, x).await? {
            Some(mut data) => {
                data.description = data.description.chars().take(128).collect();
                data.lyrics.clear();
                songs.push(data);
            }
            None => {
                bail!("get_recommend got none during getting song({x})")
            }
        };
    }
    histogram!("recommend_random_get_from_db_duration_seconds").record(start.elapsed().as_secs_f64());
    Ok(songs)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotWeeklyRedisCache {
    pub songs: Vec<PublicSongDetail>,
    pub create_time: DateTime<Utc>,
}

pub async fn get_hot_songs(redis: &ConnectionManager, pool: &Pool<Postgres>, day_delta: i64, limit: i64) -> anyhow::Result<Vec<PublicSongDetail>> {
    let cache = get_from_cache_hot(redis.clone(), day_delta, limit).await?;
    if let Some(cache) = cache {
        return Ok(cache);
    }

    let songs = get_from_db_hot_weekly(redis, pool, day_delta, limit).await?;
    save_cache_hot(redis.clone(), &songs, day_delta, limit).await?;
    Ok(songs)
}

async fn get_from_cache_hot(mut redis: ConnectionManager, day_delta: i64, limit: i64) -> anyhow::Result<Option<Vec<PublicSongDetail>>> {
    let cache: Option<String> = redis.get(format!("songs:hot:{}:{}", day_delta, limit)).await?;
    match cache {
        Some(cache) => match serde_json::from_str::<HotWeeklyRedisCache>(&cache) {
            Ok(x) => Ok(Some(x.songs)),
            Err(e) => {
                warn!("Got hot weekly songs data from cache but could not be parsed: {e:?}");
                Ok(None)
            }
        },
        None => Ok(None),
    }
}

async fn save_cache_hot(mut redis: ConnectionManager, songs: &[PublicSongDetail], day_delta: i64, limit: i64) -> anyhow::Result<()> {
    let cache = HotWeeklyRedisCache {
        songs: songs.to_vec(),
        create_time: Utc::now(),
    };
    let value = serde_json::to_string(&cache)?;

    // Cache for 1 hour
    let _: () = redis.set_ex(format!("songs:hot:{}:{}", day_delta, limit), value, 3600).await?;
    Ok(())
}

async fn get_from_db_hot_weekly(redis: &ConnectionManager, pool: &Pool<Postgres>, day_delta: i64, limit: i64) -> anyhow::Result<Vec<PublicSongDetail>> {
    let time_ago = Utc::now().sub(TimeDelta::days(day_delta));
    let result = sqlx::query!("
        SELECT s.title, sp.song_id, count(*) AS play_count
        FROM song_plays sp
                 JOIN songs s ON sp.song_id = s.id
        WHERE sp.create_time > $1
        GROUP BY sp.song_id, s.title
        ORDER BY play_count DESC
        LIMIT $2
    ", time_ago, limit).fetch_all(pool).await?;

    let mut songs = vec![];
    for x in result {
        if let Some(x) = get_public_detail_with_cache(redis.clone(), pool, x.song_id).await? {
            songs.push(x);
        } else {
            warn!("get_weekly_hot_songs got none during getting song({})", x.song_id)
        }
    }
    Ok(songs)
}