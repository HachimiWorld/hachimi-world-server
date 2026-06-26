use crate::db::follow::FollowDao;
use crate::db::user::UserDao;
use crate::db::CrudDao;
use crate::service::errors::{ServiceError, ServiceResult};
use redis::aio::ConnectionManager;
use redis::AsyncTypedCommands;
use sqlx::PgPool;
use std::collections::HashSet;
use tracing::info;

/// Typed errors for follow/unfollow operations.
#[derive(Debug, thiserror::Error)]
pub enum FollowError {
    #[error("cannot follow yourself")]
    CannotFollowYourself,
    #[error("cannot unfollow yourself")]
    CannotUnfollowYourself,
    #[error("target user not found")]
    TargetUserNotFound,
    #[error("target user is banned")]
    TargetUserBanned,
}

fn gen_profile_cache_key(uid: i64) -> String {
    format!("user_profile:uid={}", uid)
}

fn gen_follow_cache_key(follower_id: i64, followed_id: i64) -> String {
    format!("user:follow:{}:{}", follower_id, followed_id)
}

/// Follow a user. Returns the target's new follower_count.
pub async fn follow_user(
    mut redis: ConnectionManager,
    pool: &PgPool,
    my_uid: i64,
    target_uid: i64,
) -> ServiceResult<i64, FollowError> {
    if my_uid == target_uid {
        return Err(ServiceError::BusinessError(FollowError::CannotFollowYourself));
    }

    // Check target exists and is not banned
    let target = UserDao::get_by_id(pool, target_uid).await?
        .ok_or_else(|| ServiceError::BusinessError(FollowError::TargetUserNotFound))?;
    if target.is_banned {
        return Err(ServiceError::BusinessError(FollowError::TargetUserBanned));
    }

    // Check if already following (idempotent)
    if FollowDao::exists(pool, my_uid, target_uid).await? {
        let updated_target = UserDao::get_by_id(pool, target_uid).await?
            .ok_or_else(|| ServiceError::BusinessError(FollowError::TargetUserNotFound))?;
        return Ok(updated_target.follower_count.unwrap_or(0));
    }

    let mut tx = pool.begin().await?;

    match FollowDao::insert(&mut *tx, my_uid, target_uid).await {
        Ok(_) => {}
        Err(e) => {
            if let Some(db_err) = e.as_database_error() {
                if db_err.constraint() == Some("idx_follows_follower_followed") {
                    tx.rollback().await?;
                    let updated_target = UserDao::get_by_id(pool, target_uid).await?
                        .ok_or_else(|| ServiceError::BusinessError(FollowError::TargetUserNotFound))?;
                    return Ok(updated_target.follower_count.unwrap_or(0));
                }
            }
            tx.rollback().await?;
            return Err(e.into());
        }
    }

    FollowDao::increase_follower_count(&mut *tx, target_uid, 1).await?;
    FollowDao::increase_following_count(&mut *tx, my_uid, 1).await?;

    tx.commit().await?;

    let cache_keys: Vec<String> = vec![gen_profile_cache_key(my_uid), gen_profile_cache_key(target_uid)];
    let _ = redis.del(&cache_keys).await?;
    let _: () = redis.set_ex(gen_follow_cache_key(my_uid, target_uid), true, 600).await?;

    let updated_target = UserDao::get_by_id(pool, target_uid).await?
        .ok_or_else(|| ServiceError::BusinessError(FollowError::TargetUserNotFound))?;
    let follower_count = updated_target.follower_count.unwrap_or(0);

    metrics::counter!("follow_total").increment(1);
    info!(follower_id = my_uid, followed_id = target_uid, "user followed");

    Ok(follower_count)
}

/// Unfollow a user. Returns the target's new follower_count.
pub async fn unfollow_user(
    mut redis: ConnectionManager,
    pool: &PgPool,
    my_uid: i64,
    target_uid: i64,
) -> ServiceResult<i64, FollowError> {
    if my_uid == target_uid {
        return Err(ServiceError::BusinessError(FollowError::CannotUnfollowYourself));
    }

    if !FollowDao::exists(pool, my_uid, target_uid).await? {
        let target = UserDao::get_by_id(pool, target_uid).await?
            .ok_or_else(|| ServiceError::BusinessError(FollowError::TargetUserNotFound))?;
        return Ok(target.follower_count.unwrap_or(0));
    }

    let mut tx = pool.begin().await?;

    FollowDao::delete(&mut *tx, my_uid, target_uid).await?;
    FollowDao::increase_follower_count(&mut *tx, target_uid, -1).await?;
    FollowDao::increase_following_count(&mut *tx, my_uid, -1).await?;
    tx.commit().await?;

    let _ = redis.del(&[gen_profile_cache_key(my_uid), gen_profile_cache_key(target_uid)]).await?;
    let _ = redis.del(&[gen_follow_cache_key(my_uid, target_uid)]).await?;

    let updated_target = UserDao::get_by_id(pool, target_uid).await?
        .ok_or_else(|| ServiceError::BusinessError(FollowError::TargetUserNotFound))?;
    let follower_count = updated_target.follower_count.unwrap_or(0);

    metrics::counter!("unfollow_total").increment(1);
    info!(follower_id = my_uid, followed_id = target_uid, "user unfollowed");

    Ok(follower_count)
}

/// Get the following list with cursor pagination.
pub async fn get_following(
    pool: &PgPool,
    my_uid: i64,
    cursor: Option<&str>,
    limit: i64,
) -> anyhow::Result<(Vec<FollowingItem>, Option<String>)> {
    let cursor_dt = cursor
        .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    let mut rows = FollowDao::list_following(pool, my_uid, cursor_dt, limit + 1).await?;
    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }

    let followed_ids: Vec<i64> = rows.iter().map(|r| r.uid).collect();

    // Hydrate mutual status
    let mutual_ids_set: HashSet<i64> = FollowDao::filter_following_back(pool, my_uid, &followed_ids).await?;

    let items: Vec<FollowingItem> = rows
        .into_iter()
        .map(|r| FollowingItem {
            uid: r.uid,
            username: r.username,
            avatar_url: r.avatar_url,
            bio: r.bio,
            is_unavailable: r.is_banned,
            is_mutual: mutual_ids_set.contains(&r.uid),
            followed_at: r.followed_at.to_rfc3339(),
        })
        .collect();

    let next_cursor = if has_more {
        items.last().map(|i| i.followed_at.clone())
    } else {
        None
    };

    Ok((items, next_cursor))
}

/// Get the followers list with cursor pagination.
pub async fn get_followers(
    pool: &PgPool,
    my_uid: i64,
    cursor: Option<&str>,
    limit: i64,
) -> anyhow::Result<(Vec<FollowerItem>, Option<String>)> {
    let cursor_dt = cursor
        .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    let mut rows = FollowDao::list_followers(pool, my_uid, cursor_dt, limit + 1).await?;
    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }

    let follower_ids: Vec<i64> = rows.iter().map(|r| r.uid).collect();

    let mutual_ids = FollowDao::filter_following_back(pool, my_uid, &follower_ids).await?;
    let mutual_ids_set: HashSet<i64> = mutual_ids.into_iter().collect();

    let items: Vec<FollowerItem> = rows
        .into_iter()
        .map(|r| FollowerItem {
            uid: r.uid,
            username: r.username,
            avatar_url: r.avatar_url,
            bio: r.bio,
            is_unavailable: r.is_banned,
            is_mutual: mutual_ids_set.contains(&r.uid),
            followed_at: r.followed_at.to_rfc3339(),
        })
        .collect();

    let next_cursor = if has_more {
        items.last().map(|i| i.followed_at.clone())
    } else {
        None
    };

    Ok((items, next_cursor))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FollowingItem {
    pub uid: i64,
    pub username: String,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub is_unavailable: bool,
    pub is_mutual: bool,
    pub followed_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FollowerItem {
    pub uid: i64,
    pub username: String,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub is_unavailable: bool,
    pub is_mutual: bool,
    pub followed_at: String,
}

/// Check follow relationship between two users.
/// Returns (is_following, is_followed_by).
pub async fn check_follow_relationship(
    pool: &PgPool,
    viewer_uid: i64,
    profile_uid: i64,
) -> anyhow::Result<(bool, bool)> {
    if viewer_uid == profile_uid {
        return Ok((false, false));
    }

    let is_following = FollowDao::exists(pool, viewer_uid, profile_uid).await?;
    let is_followed_by = FollowDao::exists(pool, profile_uid, viewer_uid).await?;

    Ok((is_following, is_followed_by))
}
