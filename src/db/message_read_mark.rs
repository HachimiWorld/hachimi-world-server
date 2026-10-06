use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

/// @since 261006
#[derive(sqlx::FromRow)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageReadMark {
    pub uid: i64,
    /// `like` or `follow`
    pub channel: String,
    /// Messages after this are unread.
    pub read_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub struct MessageReadMarkDao;

impl MessageReadMarkDao {
    pub async fn list_by_uid<'e>(executor: impl PgExecutor<'e>, uid: i64) -> sqlx::Result<Vec<MessageReadMark>> {
        sqlx::query_as!(
            MessageReadMark,
            "SELECT uid, channel, read_time, update_time FROM message_read_marks WHERE uid = $1",
            uid
        ).fetch_all(executor).await
    }

    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &MessageReadMark) -> sqlx::Result<()> {
        sqlx::query!(
            "INSERT INTO message_read_marks(uid, channel, read_time, update_time) VALUES($1, $2, $3, $4)",
            value.uid, value.channel, value.read_time, value.update_time
        ).execute(executor).await?;
        Ok(())
    }

    /// Returns whether a row was updated.
    pub async fn update<'e>(executor: impl PgExecutor<'e>, value: &MessageReadMark) -> sqlx::Result<bool> {
        sqlx::query!(
            "UPDATE message_read_marks SET read_time = $3, update_time = $4 WHERE uid = $1 AND channel = $2",
            value.uid, value.channel, value.read_time, value.update_time
        ).execute(executor).await.map(|x| x.rows_affected() > 0)
    }
}
