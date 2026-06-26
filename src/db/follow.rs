use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, Result};
use std::collections::HashSet;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Follow {
    pub follower_id: i64,
    pub followed_id: i64,
    pub create_time: DateTime<Utc>,
}

/// Row type for listing users that a follower follows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowingRow {
    pub uid: i64,
    pub username: String,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub is_banned: bool,
    pub followed_at: DateTime<Utc>,
}

/// Row type for listing followers of a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowerRow {
    pub uid: i64,
    pub username: String,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub is_banned: bool,
    pub followed_at: DateTime<Utc>,
}

pub struct FollowDao;

impl FollowDao {
    pub async fn insert<'e, E>(executor: E, follower_id: i64, followed_id: i64) -> Result<Follow>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query_as!(
            Follow,
            r#"INSERT INTO follows (follower_id, followed_id) VALUES ($1, $2) RETURNING follower_id, followed_id, create_time"#,
            follower_id,
            followed_id
        )
            .fetch_one(executor)
            .await
    }

    pub async fn delete<'e, E>(executor: E, follower_id: i64, followed_id: i64) -> Result<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            "DELETE FROM follows WHERE follower_id = $1 AND followed_id = $2",
            follower_id,
            followed_id
        )
            .execute(executor)
            .await?;
        Ok(())
    }

    pub async fn exists<'e, E>(executor: E, follower_id: i64, followed_id: i64) -> Result<bool>
    where
        E: PgExecutor<'e>,
    {
        let result = sqlx::query!(
            r#"SELECT 1 as "exists: bool" FROM follows WHERE follower_id = $1 AND followed_id = $2"#,
            follower_id,
            followed_id
        )
            .fetch_optional(executor)
            .await?;
        Ok(result.is_some())
    }

    /// Get the list of users that `follower_id` is following, with cursor pagination.
    pub async fn list_following<'e, E>(
        executor: E,
        follower_id: i64,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<FollowingRow>>
    where
        E: PgExecutor<'e>,
    {
        let cursor = cursor.unwrap_or(DateTime::<Utc>::MAX_UTC);
        sqlx::query_as!(
            FollowingRow,
            r#"SELECT u.id as uid, u.username, u.avatar_url, u.bio, u.is_banned, f.create_time as followed_at
               FROM follows f
               JOIN users u ON f.followed_id = u.id
               WHERE f.follower_id = $1 AND f.create_time < $2
               ORDER BY f.create_time DESC
               LIMIT $3"#,
            follower_id,
            cursor,
            limit
        )
            .fetch_all(executor)
            .await
    }

    /// Get the list of followers of `followed_id`, with cursor pagination.
    pub async fn list_followers<'e, E>(
        executor: E,
        followed_id: i64,
        cursor: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<FollowerRow>>
    where
        E: PgExecutor<'e>,
    {
        let cursor = cursor.unwrap_or(DateTime::<Utc>::MAX_UTC);
        sqlx::query_as!(
            FollowerRow,
            r#"SELECT u.id as uid, u.username, u.avatar_url, u.bio, u.is_banned, f.create_time as followed_at
               FROM follows f
               JOIN users u ON f.follower_id = u.id
               WHERE f.followed_id = $1 AND f.create_time < $2
               ORDER BY f.create_time DESC
               LIMIT $3"#,
            followed_id,
            cursor,
            limit
        )
            .fetch_all(executor)
            .await
    }

    pub async fn increase_following_count<'e, E>(executor: E, uid: i64, delta: i64) -> Result<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            "UPDATE users SET following_count = COALESCE(following_count, 0) + $1 WHERE id = $2",
            delta, uid
        ).execute(executor).await?;
        Ok(())
    }

    pub async fn increase_follower_count<'e, E>(executor: E, uid: i64, delta: i64) -> Result<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query!(
            "UPDATE users SET follower_count = COALESCE(follower_count, 0) + $1 WHERE id = $2",
            delta, uid
        ).execute(executor).await?;
        Ok(())
    }

    // Given a list of candidate user IDs, return which ones are also following back `uid`.
    // "following back" = candidate follows uid, i.e., candidate appears as follower_id
    // where followed_id = uid.
    pub async fn filter_following_back<'e, E>(executor: E, uid: i64, candidate_ids: &[i64]) -> Result<HashSet<i64>>
    where
        E: PgExecutor<'e>,
    {
        if candidate_ids.is_empty() {
            return Ok(HashSet::new());
        }
        let rows = sqlx::query!(
            r#"SELECT follower_id FROM follows WHERE followed_id = $1 AND follower_id = ANY($2)"#,
            uid,
            candidate_ids
        ).fetch_all(executor).await?;
        Ok(rows.into_iter().map(|r| r.follower_id).collect())
    }
}
