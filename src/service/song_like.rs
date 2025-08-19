use chrono::Utc;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sqlx::PgPool;
use crate::db::song::{ISongDao, SongDao, SongLike};

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

    let likes_db = SongDao::new(sql_pool.clone()).count_likes(song_id).await?;
    set_likes_cache(&mut redis, song_id, likes_db).await?;
    Ok(likes_db)
}

pub async fn like(
    redis_conn: &ConnectionManager,
    sql_pool: &PgPool,
    song_id: i64, uid: i64,
) -> anyhow::Result<()> {
    let mut redis = redis_conn.clone();
    let db = SongDao::new(sql_pool.clone());

    let cache_is_liked: Option<bool> = redis.get(format!("song:liked:{}:{}", uid, song_id)).await?;
    match cache_is_liked {
        Some(x) => {
            if x {
                return Ok(())
            }
        }
        None => {
            let db_is_liked = db.is_liked(song_id, uid).await?;
            let _: () = redis.set(format!("song:liked:{}:{}", uid, song_id), false).await?;

            if db_is_liked {
                return Ok(())
            }
        }
    }

    let _: () = redis.set(format!("song:liked:{}:{}", uid, song_id), true).await?;
    db.insert_likes(&[SongLike {
        song_id,
        user_id: uid,
        create_time: Utc::now(),
    }]).await?;
    incr_likes_cache(&mut redis, song_id, 1).await?;
    Ok(())
}

pub async fn unlike(
    redis_conn: &ConnectionManager,
    sql_pool: &PgPool,
    song_id: i64, uid: i64,
) -> anyhow::Result<()> {
    let mut redis = redis_conn.clone();
    let db = SongDao::new(sql_pool.clone());

    let cache_is_liked: Option<bool> = redis.get(format!("song:liked:{}:{}", uid, song_id)).await?;
    match cache_is_liked {
        Some(x) => {
            if !x {
                return Ok(())
            }
        }
        None => {
            let db_is_liked = db.is_liked(song_id, uid).await?;
            let _: () = redis.set(format!("song:liked:{}:{}", uid, song_id), false).await?;

            if !db_is_liked {
                return Ok(())
            }
        }
    }

    let _: () = redis.set(format!("song:liked:{}:{}", uid, song_id), true).await?;
    db.delete_like(song_id, uid).await?;
    incr_likes_cache(&mut redis, song_id, -1).await?;
    Ok(())
}

async fn get_likes_cache(redis: &mut ConnectionManager, song_id: i64) -> anyhow::Result<Option<i64>> {
    Ok(redis.get(format!("song:likes:{}", song_id)).await?)
}

async fn set_likes_cache(redis: &mut ConnectionManager, song_id: i64, value: i64) -> anyhow::Result<()> {
    let _: () = redis.set(format!("song:likes:{}", song_id), value).await?;
    Ok(())
}

async fn incr_likes_cache(redis: &mut ConnectionManager, song_id: i64, delta: i32) -> anyhow::Result<()> {
    let _: () = redis.incr(format!("song:likes:{}", song_id), delta).await?;
    Ok(())
}