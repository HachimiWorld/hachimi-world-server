use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;
use uuid::Uuid;

macro_rules! query_email_outbox {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            EmailOutbox, r#"
            SELECT
                id,
                to_address,
                subject,
                body,
                available_time,
                attempt_count,
                sent_time,
                dead_time,
                last_error,
                create_time
            FROM email_outbox
            "# + $extra,
            $($arg),*
        )
    };
}

/// @since 261006
#[derive(sqlx::FromRow)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailOutbox {
    /// UUIDv7
    pub id: Uuid,
    pub to_address: String,
    pub subject: String,
    /// Plain text
    pub body: String,
    /// Not sent before this.
    pub available_time: DateTime<Utc>,
    pub attempt_count: i32,
    pub sent_time: Option<DateTime<Utc>>,
    /// When it was given up.
    pub dead_time: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub create_time: DateTime<Utc>,
}

pub struct EmailOutboxDao;

impl EmailOutboxDao {
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &EmailOutbox) -> sqlx::Result<()> {
        sqlx::query!("INSERT INTO email_outbox(
                id,
                to_address,
                subject,
                body,
                available_time,
                attempt_count,
                sent_time,
                dead_time,
                last_error,
                create_time
            ) VALUES($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            value.id,
            value.to_address,
            value.subject,
            value.body,
            value.available_time,
            value.attempt_count,
            value.sent_time,
            value.dead_time,
            value.last_error,
            value.create_time
        ).execute(executor).await?;
        Ok(())
    }

    pub async fn get_by_id<'e>(executor: impl PgExecutor<'e>, id: Uuid) -> sqlx::Result<Option<EmailOutbox>> {
        query_email_outbox!("WHERE id = $1", id)
            .fetch_optional(executor)
            .await
    }

    /// Neither sent nor dead, with `available_time` at or before `time`. Earliest first.
    pub async fn list_pending_available_at<'e>(
        executor: impl PgExecutor<'e>,
        time: DateTime<Utc>,
        limit: i64,
    ) -> sqlx::Result<Vec<EmailOutbox>> {
        query_email_outbox!(
            "WHERE sent_time IS NULL AND dead_time IS NULL AND available_time <= $1
            ORDER BY available_time, id
            LIMIT $2",
            time, limit
        ).fetch_all(executor).await
    }

    pub async fn exists_pending_available_at<'e>(executor: impl PgExecutor<'e>, time: DateTime<Utc>) -> sqlx::Result<bool> {
        sqlx::query!(
            r#"SELECT EXISTS(
                SELECT 1 FROM email_outbox
                WHERE sent_time IS NULL AND dead_time IS NULL AND available_time <= $1
            ) AS "exists!""#,
            time
        ).fetch_one(executor).await.map(|x| x.exists)
    }

    /// Writes the delivery fields: everything except the email itself.
    pub async fn update_delivery<'e>(executor: impl PgExecutor<'e>, value: &EmailOutbox) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE email_outbox SET
                available_time = $2,
                attempt_count = $3,
                sent_time = $4,
                dead_time = $5,
                last_error = $6
            WHERE id = $1",
            value.id,
            value.available_time,
            value.attempt_count,
            value.sent_time,
            value.dead_time,
            value.last_error
        ).execute(executor).await?;
        Ok(())
    }

    /// Deletes up to `limit` emails sent before `before`. Returns how many were deleted.
    pub async fn delete_sent_before<'e>(executor: impl PgExecutor<'e>, before: DateTime<Utc>, limit: i64) -> sqlx::Result<u64> {
        sqlx::query!(
            "DELETE FROM email_outbox WHERE id IN (
                SELECT id FROM email_outbox WHERE sent_time < $1 LIMIT $2
            )",
            before, limit
        ).execute(executor).await.map(|x| x.rows_affected())
    }

    /// Deletes up to `limit` emails given up before `before`. Returns how many were deleted.
    pub async fn delete_dead_before<'e>(executor: impl PgExecutor<'e>, before: DateTime<Utc>, limit: i64) -> sqlx::Result<u64> {
        sqlx::query!(
            "DELETE FROM email_outbox WHERE id IN (
                SELECT id FROM email_outbox WHERE dead_time < $1 LIMIT $2
            )",
            before, limit
        ).execute(executor).await.map(|x| x.rows_affected())
    }
}
