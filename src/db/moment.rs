use crate::db::user::UserId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgExecutor;

define_id!(
    /// `moments.id`
    MomentId
);

macro_rules! query_moments {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            Moment, r#"
            SELECT
                m.id as "id: _",
                m.user_id as "user_id: _",
                m.content,
                m.images,
                m.attachments,
                m.is_hidden,
                m.like_count,
                m.delete_time,
                m.create_time,
                m.update_time
            FROM moments m
            "# + $extra,
            $($arg),*
        )
    };
}

/// A moment: a short post by any user, with images and attachments. Only moments that passed their
/// content checks, or were published without them, are saved.
/// @since 261106
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Moment {
    pub id: MomentId,
    pub user_id: UserId,
    /// May be empty when there are images or attachments.
    pub content: String,
    /// `[MomentImage]`, in order.
    pub images: Value,
    /// `[{ type, data }]`, in order; each attachment type reads its own `data`.
    pub attachments: Value,
    /// Hidden by a report decision.
    pub is_hidden: bool,
    /// Derived: how many rows `moment_likes` has for it, which is the source of truth. Changed
    /// with the like or unlike, in the same transaction.
    pub like_count: i32,
    pub delete_time: Option<DateTime<Utc>>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

impl Moment {
    /// Shown to everyone: not hidden and not deleted.
    pub fn is_visible(&self) -> bool {
        !self.is_hidden && self.delete_time.is_none()
    }
}

pub struct MomentDao;

/// Where a page continues before: the previous page's last moment, or for the first page, a point
/// after every moment.
///
/// Always a value, never `$1 IS NULL OR (create_time, id) < ($1, $2)` in SQL: once Postgres uses a
/// generic plan for the prepared statement, the OR makes the cursor a filter instead of an index
/// condition, and a deep page reads every row before it, as OFFSET would.
fn before_or_end(before: Option<(DateTime<Utc>, MomentId)>) -> (DateTime<Utc>, MomentId) {
    before.unwrap_or((DateTime::<Utc>::MAX_UTC, MomentId(i64::MAX)))
}

// "Visible" in the queries below means not hidden and not deleted, as in [Moment::is_visible].
impl MomentDao {
    /// The `id` of `value` is ignored; returns the new id.
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &Moment) -> sqlx::Result<MomentId> {
        sqlx::query_scalar!(
            r#"INSERT INTO moments (user_id, content, images, attachments, is_hidden, like_count, delete_time, create_time, update_time)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id AS "id: MomentId""#,
            value.user_id as UserId, value.content, value.images, value.attachments,
            value.is_hidden, value.like_count, value.delete_time, value.create_time, value.update_time
        ).fetch_one(executor).await
    }

    pub async fn get_by_id<'e>(executor: impl PgExecutor<'e>, id: MomentId) -> sqlx::Result<Option<Moment>> {
        query_moments!("WHERE m.id = $1", id as MomentId)
            .fetch_optional(executor).await
    }

    pub async fn list_by_ids<'e>(executor: impl PgExecutor<'e>, ids: &[MomentId]) -> sqlx::Result<Vec<Moment>> {
        if ids.is_empty() { return Ok(vec![]) }
        query_moments!("WHERE m.id = ANY($1)", ids as &[MomentId])
            .fetch_all(executor).await
    }

    /// Writes what decides whether it's visible, `is_hidden` and `delete_time`, with `update_time`.
    /// Returns whether there was such a moment.
    pub async fn update_visibility<'e>(
        executor: impl PgExecutor<'e>,
        id: MomentId,
        is_hidden: bool,
        delete_time: Option<DateTime<Utc>>,
        update_time: DateTime<Utc>,
    ) -> sqlx::Result<bool> {
        sqlx::query!(
            "UPDATE moments SET is_hidden = $2, delete_time = $3, update_time = $4 WHERE id = $1",
            id as MomentId, is_hidden, delete_time, update_time
        ).execute(executor).await.map(|x| x.rows_affected() > 0)
    }

    pub async fn add_like_count<'e>(executor: impl PgExecutor<'e>, id: MomentId, delta: i32) -> sqlx::Result<()> {
        sqlx::query!("UPDATE moments SET like_count = like_count + $2 WHERE id = $1", id as MomentId, delta)
            .execute(executor).await?;
        Ok(())
    }

    /// Visible moments by users who aren't banned, newest first, continuing before `before`, the
    /// `(create_time, id)` of the previous page's last one.
    pub async fn page_public<'e>(
        executor: impl PgExecutor<'e>,
        before: Option<(DateTime<Utc>, MomentId)>,
        limit: i64,
    ) -> sqlx::Result<Vec<Moment>> {
        let (before_time, before_id) = before_or_end(before);
        query_moments!(
            "JOIN users u ON u.id = m.user_id
            WHERE NOT m.is_hidden AND m.delete_time IS NULL AND NOT u.is_banned
                AND (m.create_time, m.id) < ($1, $2)
            ORDER BY m.create_time DESC, m.id DESC
            LIMIT $3",
            before_time, before_id as MomentId, limit
        ).fetch_all(executor).await
    }

    /// Moments by `user_id`, newest first, continuing before `before`. Hidden ones only if
    /// `include_hidden`; deleted ones never.
    pub async fn page_by_user<'e>(
        executor: impl PgExecutor<'e>,
        user_id: UserId,
        include_hidden: bool,
        before: Option<(DateTime<Utc>, MomentId)>,
        limit: i64,
    ) -> sqlx::Result<Vec<Moment>> {
        let (before_time, before_id) = before_or_end(before);
        query_moments!(
            "WHERE m.user_id = $1 AND ($2 OR NOT m.is_hidden) AND m.delete_time IS NULL
                AND (m.create_time, m.id) < ($3, $4)
            ORDER BY m.create_time DESC, m.id DESC
            LIMIT $5",
            user_id as UserId, include_hidden, before_time, before_id as MomentId, limit
        ).fetch_all(executor).await
    }

    /// Moments `user_id` posted after `after`, hidden, deleted or not.
    pub async fn count_by_user_after<'e>(executor: impl PgExecutor<'e>, user_id: UserId, after: DateTime<Utc>) -> sqlx::Result<i64> {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM moments WHERE user_id = $1 AND create_time > $2"#,
            user_id as UserId, after
        ).fetch_one(executor).await
    }
}

pub struct MomentLikeDao;

impl MomentLikeDao {
    /// Returns false if `user_id` already liked it.
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, moment_id: MomentId, user_id: UserId, create_time: DateTime<Utc>) -> sqlx::Result<bool> {
        sqlx::query!(
            "INSERT INTO moment_likes (moment_id, user_id, create_time) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            moment_id as MomentId, user_id as UserId, create_time
        ).execute(executor).await.map(|x| x.rows_affected() > 0)
    }

    /// Returns whether there was a like to delete.
    pub async fn delete<'e>(executor: impl PgExecutor<'e>, moment_id: MomentId, user_id: UserId) -> sqlx::Result<bool> {
        sqlx::query!(
            "DELETE FROM moment_likes WHERE moment_id = $1 AND user_id = $2",
            moment_id as MomentId, user_id as UserId
        ).execute(executor).await.map(|x| x.rows_affected() > 0)
    }

    /// Which of `moment_ids` `user_id` liked.
    pub async fn list_liked_by_user<'e>(executor: impl PgExecutor<'e>, user_id: UserId, moment_ids: &[MomentId]) -> sqlx::Result<Vec<MomentId>> {
        if moment_ids.is_empty() { return Ok(vec![]) }
        sqlx::query_scalar!(
            r#"SELECT moment_id AS "moment_id: MomentId" FROM moment_likes WHERE user_id = $1 AND moment_id = ANY($2)"#,
            user_id as UserId, moment_ids as &[MomentId]
        ).fetch_all(executor).await
    }
}
