use crate::db::song::{ISongDao, SongDao, SongLike};
use chrono::Utc;
use itertools::Itertools;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sqlx::PgPool;
use std::collections::HashMap;

pub async fn get_song_likes_batch(
    redis_conn: ConnectionManager,
    sql_pool: &PgPool,
    song_ids: &[i64],
) -> anyhow::Result<HashMap<i64, i64>>{
    let mut redis = redis_conn.clone();
    let likes_cache = get_likes_cache_batch(&mut redis, song_ids).await?;

    let missed_ids = likes_cache.iter()
        .filter_map(|(id, cache)| cache.as_ref().map(|_| *id).or(Some(*id)))
        .collect::<Vec<_>>();
    if missed_ids.is_empty() {
        let filtered = likes_cache.into_iter()
            .filter_map(|(id, likes)|
                likes.map(|likes| (id, likes))
            ).collect();

        Ok(filtered)
    } else {
        let counts = SongDao::count_likes_batch(sql_pool, &missed_ids).await?;
        set_likes_cache_batch(&mut redis, &counts.iter().map(|(id, likes)| (*id, *likes)).collect::<Vec<_>>()).await?;
        let mut filtered = likes_cache.into_iter()
            .filter_map(|(id, likes)|
                likes.map(|likes| (id, likes))
            ).collect::<HashMap<_, _>>();
        filtered.extend(counts);
        Ok(filtered)
    }
}

pub async fn get_song_likes(
    redis_conn: &ConnectionManager,
    sql_pool: &PgPool,
    song_id: i64
) -> anyhow::Result<i64> {
    let mut redis = redis_conn.clone();
    let likes_cache = get_likes_cache(&mut redis, song_id).await?;
    if let Some(x) = likes_cache {
        return Ok(x)
    }

    let likes_db = SongDao::count_likes(sql_pool, song_id).await?;
    set_likes_cache(&mut redis, song_id, likes_db).await?;
    Ok(likes_db)
}

pub async fn like(
    redis_conn: &ConnectionManager,
    sql_pool: &PgPool,
    uid: i64, song_id: i64,
    playback_position_secs: Option<i32>
) -> anyhow::Result<()> {
    let mut redis = redis_conn.clone();

    let cache_is_liked = get_cache_is_liked(&mut redis, uid, song_id).await?;
    match cache_is_liked {
        Some(x) => {
            if x {
                return Ok(())
            }
        }
        None => {
            let db_is_liked = SongDao::is_liked(sql_pool, song_id, uid).await?;
            set_cache_is_liked(&mut redis, uid, song_id, db_is_liked).await?;

            if db_is_liked {
                return Ok(())
            }
        }
    }

    SongDao::insert_likes(sql_pool, &[SongLike {
        song_id,
        user_id: uid,
        playback_position_secs,
        create_time: Utc::now(),
    }]).await?;
    set_cache_is_liked(&mut redis, uid, song_id, true).await?;
    incr_likes_cache(&mut redis, song_id, 1).await?;
    Ok(())
}

pub async fn is_liked(
    redis_conn: &ConnectionManager,
    sql_pool: &PgPool,
    uid: i64, song_id: i64,
) -> anyhow::Result<bool> {
    let mut redis = redis_conn.clone();

    if let Some(cache_is_liked) = get_cache_is_liked(&mut redis, uid, song_id).await? {
        return Ok(cache_is_liked);
    }

    let db_is_liked = SongDao::is_liked(sql_pool, song_id, uid).await?;
    set_cache_is_liked(&mut redis, uid, song_id, db_is_liked).await?;
    Ok(db_is_liked)
}

pub async fn unlike(
    redis_conn: &ConnectionManager,
    sql_pool: &PgPool,
    uid: i64, song_id: i64,
) -> anyhow::Result<()> {
    let mut redis = redis_conn.clone();

    let cache_is_liked = get_cache_is_liked(&mut redis, uid, song_id).await?;
    match cache_is_liked {
        Some(x) => {
            if !x {
                return Ok(())
            }
        }
        None => {
            let db_is_liked = SongDao::is_liked(sql_pool, song_id, uid).await?;
            set_cache_is_liked(&mut redis, uid, song_id, db_is_liked).await?;

            if !db_is_liked {
                return Ok(())
            }
        }
    }

    SongDao::delete_like(sql_pool, song_id, uid).await?;
    set_cache_is_liked(&mut redis, uid, song_id, false).await?;
    incr_likes_cache(&mut redis, song_id, -1).await?;
    Ok(())
}

pub async fn page_by_user(
    _redis_conn: &ConnectionManager,
    sql_pool: &PgPool,
    user_id: i64,
    page_index: i64,
    page_size: i64,
) -> anyhow::Result<(i64, Vec<SongLike>)> {
    let count = SongDao::count_likes_by_user(sql_pool, user_id).await?;
    let rows = SongDao::page_likes_by_user(sql_pool, user_id, page_index, page_size).await?;
    Ok((count, rows))
}

async fn get_likes_cache(redis: &mut ConnectionManager, song_id: i64) -> anyhow::Result<Option<i64>> {
    Ok(redis.get(format!("song:likes:{}", song_id)).await?)
}

async fn get_likes_cache_batch(redis: &mut ConnectionManager, song_ids: &[i64]) -> anyhow::Result<HashMap<i64, Option<i64>>> {
    let keys: Vec<String> = song_ids.iter().map(|id| format!("song:likes:{}", id)).collect();
    let values: Vec<Option<i64>> = redis.mget(keys).await?;
    let result: HashMap<i64, Option<i64>> = song_ids.iter().cloned().zip(values.into_iter()).collect();
    Ok(result)
}

async fn set_likes_cache(redis: &mut ConnectionManager, song_id: i64, value: i64) -> anyhow::Result<()> {
    let _: () = redis.set_ex(format!("song:likes:{}", song_id), value, 300).await?;
    Ok(())
}

async fn set_likes_cache_batch(redis: &mut ConnectionManager, values: &[(i64, i64)]) -> anyhow::Result<()> {
    let entries = values.iter().map(|(id, likes)|
        (format!("song:likes:{}", id), likes)
    ).collect_vec();
    let _: () = redis.mset(&entries).await?;
    Ok(())
}

async fn incr_likes_cache(redis: &mut ConnectionManager, song_id: i64, delta: i32) -> anyhow::Result<()> {
    let _: () = redis.incr(format!("song:likes:{}", song_id), delta).await?;
    Ok(())
}

async fn get_cache_is_liked(redis: &mut ConnectionManager, uid: i64, song_id: i64) -> anyhow::Result<Option<bool>> {
    Ok(redis.get(format!("song:liked:{}:{}", uid, song_id)).await?)
}

async fn set_cache_is_liked(redis: &mut ConnectionManager, uid: i64, song_id: i64, value: bool) -> anyhow::Result<()> {
    let _: () = redis.set(format!("song:liked:{}:{}", uid, song_id), value).await?;
    Ok(())
}