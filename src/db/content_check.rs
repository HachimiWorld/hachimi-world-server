use chrono::{DateTime, Utc};
use crate::db::user::UserId;
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

macro_rules! query_content_policies {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            ContentPolicy, r#"
            SELECT
                version,
                files,
                create_time
            FROM content_policies
            "# + $extra,
            $($arg),*
        )
    };
}

macro_rules! query_content_checks {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            ContentCheck, r#"
            SELECT
                id,
                subject,
                target_id,
                actor_uid as "actor_uid: _",
                content,
                content_hash,
                policy_version,
                model,
                answers,
                error,
                blocked_by,
                latency_ms,
                reused_from,
                create_time
            FROM content_checks
            "# + $extra,
            $($arg),*
        )
    };
}

/// A version of the content check policy, kept so checks can be traced to what judged them.
/// @since 261016
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPolicy {
    /// First 12 hex digits of the sha256 of the policy directory. Not ordered; sort by `create_time`.
    pub version: String,
    /// Relative path -> file content.
    pub files: serde_json::Value,
    pub create_time: DateTime<Utc>,
}

/// One piece of content checked (a text or an image), passed or blocked.
/// @since 261016
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentCheck {
    pub id: i64,
    /// What was checked, such as `user.username` or `user.bio`.
    pub subject: String,
    /// The object the content belongs to.
    pub target_id: i64,
    pub actor_uid: UserId,
    /// The text as submitted, or the storage key of an image; which one depends on `subject`.
    pub content: String,
    /// sha256 hex of what the user submitted: the text as it is, or the image file as uploaded
    /// (not the copy re-encoded for the check).
    pub content_hash: String,
    pub policy_version: String,
    /// The model that answered; `None` when the call failed.
    pub model: Option<String>,
    /// The answers it returned; `None` when the call failed.
    pub answers: Option<serde_json::Value>,
    /// Why the call failed; `None` when it succeeded.
    pub error: Option<String>,
    /// Questions that reached their threshold; empty when passed.
    pub blocked_by: Vec<String>,
    /// 0 when reused.
    pub latency_ms: i32,
    /// The check whose answer this one reused, always one that asked TypeSafe; `None` when this one
    /// asked TypeSafe itself.
    /// @since 261019
    pub reused_from: Option<i64>,
    pub create_time: DateTime<Utc>,
}

pub struct ContentPolicyDao;

impl ContentPolicyDao {
    pub async fn get_by_version<'e>(executor: impl PgExecutor<'e>, version: &str) -> sqlx::Result<Option<ContentPolicy>> {
        query_content_policies!("WHERE version = $1", version)
            .fetch_optional(executor).await
    }

    /// Does nothing if the version is already stored.
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &ContentPolicy) -> sqlx::Result<()> {
        sqlx::query!(
            "INSERT INTO content_policies (version, files, create_time) VALUES ($1, $2, $3) ON CONFLICT (version) DO NOTHING",
            value.version, value.files, value.create_time
        ).execute(executor).await?;
        Ok(())
    }
}

pub struct ContentCheckDao;

impl ContentCheckDao {
    /// The `id` of `value` is ignored; returns the new id.
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &ContentCheck) -> sqlx::Result<i64> {
        sqlx::query_scalar!(
            "INSERT INTO content_checks (subject, target_id, actor_uid, content, content_hash, policy_version, model, answers, error, blocked_by, latency_ms, reused_from, create_time)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) RETURNING id",
            value.subject, value.target_id, value.actor_uid as UserId, value.content, value.content_hash, value.policy_version,
            value.model, value.answers, value.error, &value.blocked_by, value.latency_ms, value.reused_from, value.create_time
        ).fetch_one(executor).await
    }

    /// Newest first.
    pub async fn list_by_actor<'e>(executor: impl PgExecutor<'e>, actor_uid: UserId, limit: i64) -> sqlx::Result<Vec<ContentCheck>> {
        query_content_checks!("WHERE actor_uid = $1 ORDER BY create_time DESC, id DESC LIMIT $2", actor_uid as UserId, limit)
            .fetch_all(executor).await
    }

    /// The latest check of `content_hash` as `subject` under `policy_version` whose call succeeded
    /// and was answered by `model`.
    /// @since 261019
    pub async fn get_latest_succeeded<'e>(executor: impl PgExecutor<'e>, subject: &str, content_hash: &str, policy_version: &str, model: &str) -> sqlx::Result<Option<ContentCheck>> {
        query_content_checks!(
            "WHERE content_hash = $1 AND subject = $2 AND policy_version = $3 AND error IS NULL AND model = $4 ORDER BY id DESC LIMIT 1",
            content_hash, subject, policy_version, model
        ).fetch_optional(executor).await
    }

    /// Counts only checks that asked TypeSafe, not reused ones.
    /// @since 261018
    pub async fn count_by_actor_and_subject_since<'e>(executor: impl PgExecutor<'e>, actor_uid: UserId, subject: &str, since: DateTime<Utc>) -> sqlx::Result<i64> {
        sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM content_checks WHERE actor_uid = $1 AND subject = $2 AND create_time > $3 AND reused_from IS NULL"#,
            actor_uid as UserId, subject, since
        ).fetch_one(executor).await
    }
}
