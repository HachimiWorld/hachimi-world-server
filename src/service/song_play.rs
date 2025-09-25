use std::time::Duration;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use sqlx::PgPool;
use crate::db::song::{ISongDao, SongDao};
use crate::service::errors::ServiceResult;
use crate::util::redlock::RedLock;

pub async fn get_play_count(
    redis: &mut ConnectionManager,
    red_lock: &RedLock,
    sql_pool: &PgPool,
    song_id: i64
) -> ServiceResult<i64, ()> {
    let cache = get_plays_cache(redis, song_id).await?;
    if let Some(x) = cache {
        return Ok(x)
    }

    let guard = red_lock.lock_with_timeout("lock:song_plays", Duration::from_secs(10)).await?;
    let cache = get_plays_cache(redis, song_id).await?;
    if let Some(x) = cache {
        return Ok(x)
    }

    let likes_db = SongDao::count_plays(sql_pool, song_id).await?;
    set_plays_cache(redis, song_id, likes_db).await?;
    drop(guard);
    Ok(likes_db)
}

async fn get_plays_cache(redis: &mut ConnectionManager, song_id: i64) -> anyhow::Result<Option<i64>> {
    Ok(redis.get(format!("song:plays:{}", song_id)).await?)
}

async fn set_plays_cache(redis: &mut ConnectionManager, song_id: i64, value: i64) -> anyhow::Result<()> {
    let _: () = redis.set_ex(format!("song:likes:{}", song_id), value, 300).await?;
    Ok(())
}