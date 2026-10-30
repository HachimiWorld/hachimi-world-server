use crate::db::user::UserId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgExecutor;

macro_rules! query_content_reviews {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            ContentReview, r#"
            SELECT
                id,
                subject,
                target_id,
                actor_uid as "actor_uid: _",
                payload,
                attempt_count,
                available_time,
                done_time,
                last_error,
                create_time
            FROM content_reviews
            "# + $extra,
            $($arg),*
        )
    };
}

/// Content waiting for a content check that couldn't be done when it was submitted.
/// @since 261021
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentReview {
    pub id: i64,
    /// The content check subject, such as `song.comment`.
    pub subject: String,
    /// The id the content will have once saved, such as the comment id.
    pub target_id: i64,
    /// Who submitted it.
    pub actor_uid: UserId,
    /// The content, in the shape its kind defines, saved to its own table once it passes.
    pub payload: Value,
    pub attempt_count: i32,
    /// Not tried before this.
    pub available_time: DateTime<Utc>,
    /// When it was settled.
    pub done_time: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub create_time: DateTime<Utc>,
}

/// The reviews of one subject that are not done.
#[derive(Debug, Clone)]
pub struct PendingSummary {
    pub subject: String,
    pub count: i64,
    pub oldest_create_time: DateTime<Utc>,
}

pub struct ContentReviewDao;

impl ContentReviewDao {
    /// The `id` of `value` is ignored; returns the new id.
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &ContentReview) -> sqlx::Result<i64> {
        sqlx::query_scalar!(
            "INSERT INTO content_reviews (subject, target_id, actor_uid, payload, attempt_count, available_time, done_time, last_error, create_time)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
            value.subject, value.target_id, value.actor_uid as UserId, value.payload, value.attempt_count, value.available_time,
            value.done_time, value.last_error, value.create_time
        ).fetch_one(executor).await
    }

    /// Not done, with `available_time` at or before `time`. Earliest first.
    pub async fn list_pending_available_at<'e>(executor: impl PgExecutor<'e>, time: DateTime<Utc>, limit: i64) -> sqlx::Result<Vec<ContentReview>> {
        query_content_reviews!(
            "WHERE done_time IS NULL AND available_time <= $1 ORDER BY available_time, id LIMIT $2",
            time, limit
        ).fetch_all(executor).await
    }

    pub async fn exists_pending_available_at<'e>(executor: impl PgExecutor<'e>, time: DateTime<Utc>) -> sqlx::Result<bool> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS(SELECT 1 FROM content_reviews WHERE done_time IS NULL AND available_time <= $1) AS "exists!""#,
            time
        ).fetch_one(executor).await
    }

    /// For each subject with reviews not done: how many, and when the oldest was submitted.
    pub async fn summarize_pending<'e>(executor: impl PgExecutor<'e>) -> sqlx::Result<Vec<PendingSummary>> {
        sqlx::query_as!(
            PendingSummary,
            r#"SELECT subject, COUNT(*) AS "count!", MIN(create_time) AS "oldest_create_time!"
            FROM content_reviews WHERE done_time IS NULL GROUP BY subject"#
        ).fetch_all(executor).await
    }

    /// Not done, submitted by `actor_uid` with one of `subjects` after `after`.
    pub async fn count_pending_by_actor_after<'e>(executor: impl PgExecutor<'e>, actor_uid: UserId, subjects: &[&str], after: DateTime<Utc>) -> sqlx::Result<i64> {
        let subjects: Vec<String> = subjects.iter().map(|x| x.to_string()).collect();
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM content_reviews
            WHERE actor_uid = $1 AND create_time > $2 AND done_time IS NULL AND subject = ANY($3)"#,
            actor_uid as UserId, after, &subjects
        ).fetch_one(executor).await
    }

    pub async fn list_by_subject_and_target<'e>(executor: impl PgExecutor<'e>, subject: &str, target_id: i64) -> sqlx::Result<Vec<ContentReview>> {
        query_content_reviews!("WHERE subject = $1 AND target_id = $2 ORDER BY id", subject, target_id)
            .fetch_all(executor).await
    }

    /// Writes everything except the subject, the target, the actor, the payload and the create time.
    pub async fn update<'e>(executor: impl PgExecutor<'e>, value: &ContentReview) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE content_reviews SET attempt_count = $2, available_time = $3, done_time = $4, last_error = $5 WHERE id = $1",
            value.id, value.attempt_count, value.available_time, value.done_time, value.last_error
        ).execute(executor).await?;
        Ok(())
    }
}
