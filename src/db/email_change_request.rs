use crate::db::user::UserId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;
use uuid::Uuid;

macro_rules! query_email_change_requests {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            EmailChangeRequest, r#"
            SELECT
                id,
                user_id as "user_id: _",
                old_email,
                new_email,
                method,
                status,
                effective_time,
                revert_token_hash,
                revert_expire_time,
                reminder_email_ids,
                ip_address,
                user_agent,
                finish_time,
                create_time
            FROM email_change_requests
            "# + $extra,
            $($arg),*
        )
    };
}

/// The user proved they can read the old address. Applied at once.
pub const METHOD_OLD_EMAIL: &str = "old_email";
/// The user gave their password instead of a code from the old address. Applied after a wait.
pub const METHOD_WAIT: &str = "wait";

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_APPLIED: &str = "applied";
/// Ended before it was applied, by the user or from the old address.
pub const STATUS_CANCELLED: &str = "cancelled";
/// Undone from the old address after it was applied.
pub const STATUS_REVERTED: &str = "reverted";
/// The new address was taken, or the account email changed some other way, before it applied.
pub const STATUS_FAILED: &str = "failed";

/// @since 261105
#[derive(sqlx::FromRow)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailChangeRequest {
    pub id: i64,
    pub user_id: UserId,
    pub old_email: String,
    pub new_email: String,
    /// [METHOD_OLD_EMAIL] or [METHOD_WAIT]
    pub method: String,
    /// One of the `STATUS_` constants
    pub status: String,
    pub effective_time: DateTime<Utc>,
    /// SHA-256 of the undo token, hex.
    pub revert_token_hash: Option<String>,
    pub revert_expire_time: Option<DateTime<Utc>>,
    pub reminder_email_ids: Vec<Uuid>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub finish_time: Option<DateTime<Utc>>,
    pub create_time: DateTime<Utc>,
}

pub struct EmailChangeRequestDao;

impl EmailChangeRequestDao {
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &EmailChangeRequest) -> sqlx::Result<i64> {
        let row = sqlx::query!(
            "INSERT INTO email_change_requests(
                user_id,
                old_email,
                new_email,
                method,
                status,
                effective_time,
                revert_token_hash,
                revert_expire_time,
                reminder_email_ids,
                ip_address,
                user_agent,
                finish_time,
                create_time
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            RETURNING id",
            value.user_id as UserId,
            value.old_email,
            value.new_email,
            value.method,
            value.status,
            value.effective_time,
            value.revert_token_hash,
            value.revert_expire_time,
            &value.reminder_email_ids,
            value.ip_address,
            value.user_agent,
            value.finish_time,
            value.create_time,
        ).fetch_one(executor).await?;
        Ok(row.id)
    }

    pub async fn get_by_id<'e>(executor: impl PgExecutor<'e>, id: i64) -> sqlx::Result<Option<EmailChangeRequest>> {
        query_email_change_requests!("WHERE id = $1", id)
            .fetch_optional(executor)
            .await
    }

    /// Locks the row until the transaction ends.
    pub async fn get_by_id_for_update<'e>(executor: impl PgExecutor<'e>, id: i64) -> sqlx::Result<Option<EmailChangeRequest>> {
        query_email_change_requests!("WHERE id = $1 FOR UPDATE", id)
            .fetch_optional(executor)
            .await
    }

    /// Locks the row until the transaction ends.
    pub async fn get_by_revert_token_hash_for_update<'e>(executor: impl PgExecutor<'e>, hash: &str) -> sqlx::Result<Option<EmailChangeRequest>> {
        query_email_change_requests!("WHERE revert_token_hash = $1 FOR UPDATE", hash)
            .fetch_optional(executor)
            .await
    }

    pub async fn get_pending_by_user_id<'e>(executor: impl PgExecutor<'e>, uid: UserId) -> sqlx::Result<Option<EmailChangeRequest>> {
        query_email_change_requests!(
            "WHERE user_id = $1 AND status = 'pending' ORDER BY id DESC LIMIT 1",
            uid as UserId
        ).fetch_optional(executor).await
    }

    /// The latest request with `method` made at or after `since`, that wasn't cancelled or undone.
    pub async fn get_latest_effective_by_user_id_since<'e>(
        executor: impl PgExecutor<'e>,
        uid: UserId,
        method: &str,
        since: DateTime<Utc>,
    ) -> sqlx::Result<Option<EmailChangeRequest>> {
        query_email_change_requests!(
            "WHERE user_id = $1 AND method = $2 AND create_time >= $3 AND status IN ('pending', 'applied')
            ORDER BY create_time DESC LIMIT 1",
            uid as UserId, method, since
        ).fetch_optional(executor).await
    }

    /// Ids of pending requests whose `effective_time` is at or before `time`. Earliest first.
    pub async fn list_due_pending_ids<'e>(executor: impl PgExecutor<'e>, time: DateTime<Utc>, limit: i64) -> sqlx::Result<Vec<i64>> {
        let rows = sqlx::query!(
            "SELECT id FROM email_change_requests
            WHERE status = 'pending' AND effective_time <= $1
            ORDER BY effective_time, id
            LIMIT $2",
            time, limit
        ).fetch_all(executor).await?;
        Ok(rows.into_iter().map(|x| x.id).collect())
    }

    pub async fn update_status<'e>(
        executor: impl PgExecutor<'e>,
        id: i64,
        status: &str,
        finish_time: DateTime<Utc>,
    ) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE email_change_requests SET status = $2, finish_time = $3 WHERE id = $1",
            id, status, finish_time
        ).execute(executor).await?;
        Ok(())
    }
}
