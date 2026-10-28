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
    ($($extra:literal)+ $(, $arg:expr)* $(,)?) => {
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
                visible_reply_count,
                delete_time,
                create_time,
                update_time
            FROM comments
            "# $(+ $extra)+,
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
    /// Derived: how many rows `comment_likes` has for it, which is the source of truth. Changed
    /// with the like or unlike, in the same transaction.
    pub like_count: i32,
    /// Derived: how many replies under it are [visible](Comment::is_visible); 0 for a reply.
    /// Changed with the reply whose visibility changes, in the same transaction.
    pub visible_reply_count: i32,
    pub delete_time: Option<DateTime<Utc>>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

impl Comment {
    /// Shown to everyone: published, not hidden and not deleted.
    pub fn is_visible(&self) -> bool {
        self.status == STATUS_PUBLISHED && !self.is_hidden && self.delete_time.is_none()
    }
}

/// A comment whose derived counts differ from the counts they copy.
#[derive(Debug, Clone)]
pub struct Miscount {
    pub id: CommentId,
    pub like_count: i32,
    pub likes: i64,
    pub visible_reply_count: i32,
    pub visible_replies: i64,
}

pub struct CommentDao;

// "Shown" in the queries below means published, not hidden and not deleted, or, for `viewer_uid`,
// written by them and not deleted.
impl CommentDao {
    /// The `id` of `value` is ignored; returns the new id.
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &Comment) -> sqlx::Result<CommentId> {
        sqlx::query_scalar!(
            r#"INSERT INTO comments (target_type, target_id, user_id, root_id, reply_to_id, reply_to_uid, content, status, is_hidden, like_count, visible_reply_count, delete_time, create_time, update_time)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) RETURNING id as "id: CommentId""#,
            value.target_type, value.target_id, value.user_id as UserId, value.root_id as Option<CommentId>,
            value.reply_to_id as Option<CommentId>, value.reply_to_uid as UserId, value.content, value.status,
            value.is_hidden, value.like_count, value.visible_reply_count, value.delete_time, value.create_time, value.update_time
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

    /// Locks the row until the transaction ends, for changes that depend on its current state.
    pub async fn get_by_id_for_update<'e>(executor: impl PgExecutor<'e>, id: CommentId) -> sqlx::Result<Option<Comment>> {
        query_comments!("WHERE id = $1 FOR UPDATE", id as CommentId)
            .fetch_optional(executor).await
    }

    /// Writes what decides whether it's visible: the status, `is_hidden` and `delete_time`, with
    /// `update_time`.
    pub async fn update_visibility<'e>(executor: impl PgExecutor<'e>, value: &Comment) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE comments SET status = $2, is_hidden = $3, delete_time = $4, update_time = $5 WHERE id = $1",
            value.id as CommentId, value.status, value.is_hidden, value.delete_time, value.update_time
        ).execute(executor).await?;
        Ok(())
    }

    pub async fn add_visible_reply_count<'e>(executor: impl PgExecutor<'e>, id: CommentId, delta: i32) -> sqlx::Result<()> {
        sqlx::query!("UPDATE comments SET visible_reply_count = visible_reply_count + $2 WHERE id = $1", id as CommentId, delta)
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
        // One filter, two orders
        macro_rules! ordered_by {
            ($order:literal) => {
                query_comments!(
                    "WHERE target_type = $1 AND target_id = $2 AND root_id IS NULL AND (
                        (status = 'published' AND NOT is_hidden AND delete_time IS NULL)
                        OR (user_id = $3 AND delete_time IS NULL)
                        OR visible_reply_count > 0
                    ) "
                    $order
                    " LIMIT $4 OFFSET $5",
                    target_type, target_id, viewer_uid as Option<UserId>, limit, offset
                ).fetch_all(executor).await
            };
        }
        if by_likes {
            ordered_by!("ORDER BY like_count DESC, id DESC")
        } else {
            ordered_by!("ORDER BY id DESC")
        }
    }

    /// Published comments and replies on a target that aren't hidden or deleted.
    pub async fn count_published_by_target<'e>(executor: impl PgExecutor<'e>, target_type: &str, target_id: i64) -> sqlx::Result<i64> {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM comments
            WHERE target_type = $1 AND target_id = $2 AND status = 'published' AND NOT is_hidden AND delete_time IS NULL"#,
            target_type, target_id
        ).fetch_one(executor).await
    }

    /// The first `per_root` shown replies under each of `root_ids`, oldest first. Reads only those
    /// from `idx_comments_root`, however many replies a root has.
    pub async fn list_first_replies<'e>(
        executor: impl PgExecutor<'e>,
        root_ids: &[CommentId],
        viewer_uid: Option<UserId>,
        per_root: i64,
    ) -> sqlx::Result<Vec<Comment>> {
        if root_ids.is_empty() { return Ok(vec![]) }
        query_comments!(
            "WHERE id IN (
                SELECT r.id FROM UNNEST($1::bigint[]) AS root(id)
                CROSS JOIN LATERAL (
                    SELECT c.id FROM comments c
                    WHERE c.root_id = root.id AND c.delete_time IS NULL
                        AND ((c.status = 'published' AND NOT c.is_hidden) OR c.user_id = $2)
                    ORDER BY c.id LIMIT $3
                ) r
            )
            ORDER BY id",
            root_ids as &[CommentId], viewer_uid as Option<UserId>, per_root
        ).fetch_all(executor).await
    }

    /// How many shown replies `root_id` has.
    pub async fn count_replies<'e>(executor: impl PgExecutor<'e>, root_id: CommentId, viewer_uid: Option<UserId>) -> sqlx::Result<i64> {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM comments
            WHERE root_id = $1 AND (
                (status = 'published' AND NOT is_hidden AND delete_time IS NULL)
                OR (user_id = $2 AND delete_time IS NULL)
            )"#,
            root_id as CommentId, viewer_uid as Option<UserId>
        ).fetch_one(executor).await
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

    /// Published comments replying to `uid` by others, not hidden or deleted, posted after `after`,
    /// counted up to `limit`: reads at most that many, however many there are.
    pub async fn count_published_replies_to_after<'e>(executor: impl PgExecutor<'e>, uid: UserId, after: DateTime<Utc>, limit: i64) -> sqlx::Result<i64> {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM (
                SELECT 1 FROM comments
                WHERE reply_to_uid = $1 AND user_id <> $1 AND create_time > $2
                AND status = 'published' AND NOT is_hidden AND delete_time IS NULL
                LIMIT $3
            ) AS unread"#,
            uid as UserId, after, limit
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

/// Recounting the derived counts, for when they drift: a bug, or a change made by hand.
impl CommentDao {
    /// Comments whose derived counts differ from what they count.
    pub async fn list_miscounted<'e>(executor: impl PgExecutor<'e>) -> sqlx::Result<Vec<Miscount>> {
        sqlx::query_as!(
            Miscount,
            r#"SELECT c.id AS "id: CommentId", c.like_count, l.n AS "likes!", c.visible_reply_count, r.n AS "visible_replies!"
            FROM comments c
            CROSS JOIN LATERAL (SELECT COUNT(*) AS n FROM comment_likes WHERE comment_id = c.id) l
            CROSS JOIN LATERAL (
                SELECT COUNT(*) AS n FROM comments x
                WHERE x.root_id = c.id AND x.status = 'published' AND NOT x.is_hidden AND x.delete_time IS NULL
            ) r
            WHERE c.like_count <> l.n OR c.visible_reply_count <> r.n
            ORDER BY c.id"#
        ).fetch_all(executor).await
    }

    /// Counts again and sets the derived counts of `ids`.
    pub async fn recount<'e>(executor: impl PgExecutor<'e>, ids: &[CommentId]) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE comments c SET
                like_count = (SELECT COUNT(*) FROM comment_likes WHERE comment_id = c.id),
                visible_reply_count = (
                    SELECT COUNT(*) FROM comments x
                    WHERE x.root_id = c.id AND x.status = 'published' AND NOT x.is_hidden AND x.delete_time IS NULL
                )
            WHERE c.id = ANY($1)",
            ids as &[CommentId]
        ).execute(executor).await?;
        Ok(())
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
