use crate::db::user::UserId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

define_id!(
    /// `comments.id`
    CommentId
);

/// Waiting for a content check that couldn't be done when it was posted. Only its author sees it.
pub const STATUS_PENDING: &str = "pending";
/// Seen by everyone who can see the target.
pub const STATUS_PUBLISHED: &str = "published";
/// Blocked by a content check after it was posted. Only its author sees it.
pub const STATUS_REJECTED: &str = "rejected";

macro_rules! query_comments {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            Comment, r#"
            SELECT
                id as "id: _",
                target_type,
                target_id,
                user_id as "user_id: _",
                root_id as "root_id: _",
                reply_to_id as "reply_to_id: _",
                reply_to_uid as "reply_to_uid: _",
                content,
                status,
                is_hidden,
                like_count,
                delete_time,
                create_time,
                update_time
            FROM comments
            "# + $extra,
            $($arg),*
        )
    };
}

/// A comment on a target such as a song, or a reply under one.
/// @since 261021
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: CommentId,
    /// Such as `song`.
    pub target_type: String,
    pub target_id: i64,
    pub user_id: UserId,
    /// The top-level comment this reply is under; `None` for a top-level comment.
    pub root_id: Option<CommentId>,
    /// The comment replied to, the root or another reply under it; `None` for a top-level comment.
    pub reply_to_id: Option<CommentId>,
    /// The author of `reply_to_id`, or the target's owner for a top-level comment.
    pub reply_to_uid: UserId,
    pub content: String,
    /// [STATUS_PENDING], [STATUS_PUBLISHED] or [STATUS_REJECTED].
    pub status: String,
    /// Hidden by a report decision.
    pub is_hidden: bool,
    pub like_count: i32,
    pub delete_time: Option<DateTime<Utc>>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

/// How many replies a top-level comment has.
#[derive(Debug, Clone)]
pub struct ReplyCount {
    pub root_id: CommentId,
    pub count: i64,
}

pub struct CommentDao;

// "Shown" in the queries below means published, not hidden and not deleted, or, for `viewer_uid`,
// written by them and not deleted.
impl CommentDao {
    /// The `id` of `value` is ignored; returns the new id.
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &Comment) -> sqlx::Result<CommentId> {
        sqlx::query_scalar!(
            r#"INSERT INTO comments (target_type, target_id, user_id, root_id, reply_to_id, reply_to_uid, content, status, is_hidden, like_count, delete_time, create_time, update_time)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING id as "id: CommentId""#,
            value.target_type, value.target_id, value.user_id as UserId, value.root_id as Option<CommentId>,
            value.reply_to_id as Option<CommentId>, value.reply_to_uid as UserId, value.content, value.status,
            value.is_hidden, value.like_count, value.delete_time, value.create_time, value.update_time
        ).fetch_one(executor).await
    }

    pub async fn get_by_id<'e>(executor: impl PgExecutor<'e>, id: CommentId) -> sqlx::Result<Option<Comment>> {
        query_comments!("WHERE id = $1", id as CommentId)
            .fetch_optional(executor).await
    }

    pub async fn list_by_ids<'e>(executor: impl PgExecutor<'e>, ids: &[CommentId]) -> sqlx::Result<Vec<Comment>> {
        if ids.is_empty() { return Ok(vec![]) }
        query_comments!("WHERE id = ANY($1)", ids as &[CommentId])
            .fetch_all(executor).await
    }

    pub async fn update_status<'e>(executor: impl PgExecutor<'e>, id: CommentId, status: &str, update_time: DateTime<Utc>) -> sqlx::Result<()> {
        sqlx::query!("UPDATE comments SET status = $2, update_time = $3 WHERE id = $1", id as CommentId, status, update_time)
            .execute(executor).await?;
        Ok(())
    }

    pub async fn update_hidden<'e>(executor: impl PgExecutor<'e>, id: CommentId, is_hidden: bool, update_time: DateTime<Utc>) -> sqlx::Result<()> {
        sqlx::query!("UPDATE comments SET is_hidden = $2, update_time = $3 WHERE id = $1", id as CommentId, is_hidden, update_time)
            .execute(executor).await?;
        Ok(())
    }

    pub async fn update_delete_time<'e>(executor: impl PgExecutor<'e>, id: CommentId, delete_time: DateTime<Utc>) -> sqlx::Result<()> {
        sqlx::query!("UPDATE comments SET delete_time = $2, update_time = $2 WHERE id = $1", id as CommentId, delete_time)
            .execute(executor).await?;
        Ok(())
    }

    pub async fn add_like_count<'e>(executor: impl PgExecutor<'e>, id: CommentId, delta: i32) -> sqlx::Result<()> {
        sqlx::query!("UPDATE comments SET like_count = like_count + $2 WHERE id = $1", id as CommentId, delta)
            .execute(executor).await?;
        Ok(())
    }

    /// Top-level comments on a target that are shown, plus deleted or hidden ones that still have a
    /// shown reply, so their replies keep a place. Most liked first if `by_likes`, else newest first.
    pub async fn page_roots<'e>(
        executor: impl PgExecutor<'e>,
        target_type: &str,
        target_id: i64,
        viewer_uid: Option<UserId>,
        by_likes: bool,
        limit: i64,
        offset: i64,
    ) -> sqlx::Result<Vec<Comment>> {
        query_comments!(
            "WHERE target_type = $1 AND target_id = $2 AND root_id IS NULL AND (
                (status = 'published' AND NOT is_hidden AND delete_time IS NULL)
                OR (user_id = $3 AND delete_time IS NULL)
                OR EXISTS (
                    SELECT 1 FROM comments r WHERE r.root_id = comments.id
                    AND r.status = 'published' AND NOT r.is_hidden AND r.delete_time IS NULL
                )
            )
            ORDER BY CASE WHEN $4 THEN like_count ELSE 0 END DESC, id DESC
            LIMIT $5 OFFSET $6",
            target_type, target_id, viewer_uid as Option<UserId>, by_likes, limit, offset
        ).fetch_all(executor).await
    }

    /// Published comments and replies on a target that aren't hidden or deleted.
    pub async fn count_published_by_target<'e>(executor: impl PgExecutor<'e>, target_type: &str, target_id: i64) -> sqlx::Result<i64> {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM comments
            WHERE target_type = $1 AND target_id = $2 AND status = 'published' AND NOT is_hidden AND delete_time IS NULL"#,
            target_type, target_id
        ).fetch_one(executor).await
    }

    /// The first `per_root` shown replies under each of `root_ids`, oldest first.
    pub async fn list_first_replies<'e>(
        executor: impl PgExecutor<'e>,
        root_ids: &[CommentId],
        viewer_uid: Option<UserId>,
        per_root: i64,
    ) -> sqlx::Result<Vec<Comment>> {
        if root_ids.is_empty() { return Ok(vec![]) }
        query_comments!(
            "WHERE id IN (
                SELECT id FROM (
                    SELECT id, ROW_NUMBER() OVER (PARTITION BY root_id ORDER BY id) AS n FROM comments
                    WHERE root_id = ANY($1) AND (
                        (status = 'published' AND NOT is_hidden AND delete_time IS NULL)
                        OR (user_id = $2 AND delete_time IS NULL)
                    )
                ) x WHERE n <= $3
            )
            ORDER BY id",
            root_ids as &[CommentId], viewer_uid as Option<UserId>, per_root
        ).fetch_all(executor).await
    }

    /// How many shown replies each of `root_ids` has; roots without any are left out.
    pub async fn count_replies<'e>(executor: impl PgExecutor<'e>, root_ids: &[CommentId], viewer_uid: Option<UserId>) -> sqlx::Result<Vec<ReplyCount>> {
        if root_ids.is_empty() { return Ok(vec![]) }
        sqlx::query_as!(
            ReplyCount,
            r#"SELECT root_id AS "root_id!: CommentId", COUNT(*) AS "count!" FROM comments
            WHERE root_id = ANY($1) AND (
                (status = 'published' AND NOT is_hidden AND delete_time IS NULL)
                OR (user_id = $2 AND delete_time IS NULL)
            )
            GROUP BY root_id"#,
            root_ids as &[CommentId], viewer_uid as Option<UserId>
        ).fetch_all(executor).await
    }

    /// Shown replies under `root_id`, oldest first.
    pub async fn page_replies<'e>(
        executor: impl PgExecutor<'e>,
        root_id: CommentId,
        viewer_uid: Option<UserId>,
        limit: i64,
        offset: i64,
    ) -> sqlx::Result<Vec<Comment>> {
        query_comments!(
            "WHERE root_id = $1 AND (
                (status = 'published' AND NOT is_hidden AND delete_time IS NULL)
                OR (user_id = $2 AND delete_time IS NULL)
            )
            ORDER BY id LIMIT $3 OFFSET $4",
            root_id as CommentId, viewer_uid as Option<UserId>, limit, offset
        ).fetch_all(executor).await
    }

    /// Comments `user_id` posted after `after`, in any status and deleted or not.
    pub async fn count_by_user_after<'e>(executor: impl PgExecutor<'e>, user_id: UserId, after: DateTime<Utc>) -> sqlx::Result<i64> {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM comments WHERE user_id = $1 AND create_time > $2"#,
            user_id as UserId, after
        ).fetch_one(executor).await
    }

    /// Published comments replying to `uid` by others, not hidden or deleted, posted after `after`.
    pub async fn count_published_replies_to_after<'e>(executor: impl PgExecutor<'e>, uid: UserId, after: DateTime<Utc>) -> sqlx::Result<i64> {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM comments
            WHERE reply_to_uid = $1 AND user_id <> $1 AND create_time > $2
            AND status = 'published' AND NOT is_hidden AND delete_time IS NULL"#,
            uid as UserId, after
        ).fetch_one(executor).await
    }

    /// Published comments replying to `uid` by others, not hidden or deleted, newest first. Pass the
    /// create time and id of the previous page's last comment as `before`.
    pub async fn list_published_replies_to<'e>(
        executor: impl PgExecutor<'e>,
        uid: UserId,
        before: Option<(DateTime<Utc>, CommentId)>,
        limit: i64,
    ) -> sqlx::Result<Vec<Comment>> {
        let (before_time, before_id) = before.unzip();
        query_comments!(
            "WHERE reply_to_uid = $1 AND user_id <> $1
            AND status = 'published' AND NOT is_hidden AND delete_time IS NULL
            AND ($2::timestamptz IS NULL OR (create_time, id) < ($2, $3::bigint))
            ORDER BY create_time DESC, id DESC
            LIMIT $4",
            uid as UserId, before_time, before_id as Option<CommentId>, limit
        ).fetch_all(executor).await
    }
}

pub struct CommentLikeDao;

impl CommentLikeDao {
    /// Fails on the primary key if `user_id` already liked it.
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, comment_id: CommentId, user_id: UserId, create_time: DateTime<Utc>) -> sqlx::Result<()> {
        sqlx::query!(
            "INSERT INTO comment_likes (comment_id, user_id, create_time) VALUES ($1, $2, $3)",
            comment_id as CommentId, user_id as UserId, create_time
        ).execute(executor).await?;
        Ok(())
    }

    /// Returns whether there was a like to delete.
    pub async fn delete<'e>(executor: impl PgExecutor<'e>, comment_id: CommentId, user_id: UserId) -> sqlx::Result<bool> {
        sqlx::query!(
            "DELETE FROM comment_likes WHERE comment_id = $1 AND user_id = $2",
            comment_id as CommentId, user_id as UserId
        ).execute(executor).await.map(|x| x.rows_affected() > 0)
    }

    /// Which of `comment_ids` `user_id` liked.
    pub async fn list_liked_by_user<'e>(executor: impl PgExecutor<'e>, user_id: UserId, comment_ids: &[CommentId]) -> sqlx::Result<Vec<CommentId>> {
        if comment_ids.is_empty() { return Ok(vec![]) }
        sqlx::query_scalar!(
            r#"SELECT comment_id AS "comment_id: CommentId" FROM comment_likes WHERE user_id = $1 AND comment_id = ANY($2)"#,
            user_id as UserId, comment_ids as &[CommentId]
        ).fetch_all(executor).await
    }
}
