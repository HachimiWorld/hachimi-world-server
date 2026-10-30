use chrono::{DateTime, Utc};
use crate::db::user::UserId;
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

/// @since 261006
#[derive(sqlx::FromRow)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageReadMark {
    pub uid: UserId,
    /// `like` or `follow`
    pub channel: String,
    /// Messages after this are unread.
    pub read_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub struct MessageReadMarkDao;

impl MessageReadMarkDao {
    pub async fn list_by_uid<'e>(executor: impl PgExecutor<'e>, uid: UserId) -> sqlx::Result<Vec<MessageReadMark>> {
        sqlx::query_as!(
            MessageReadMark,
            r#"SELECT uid as "uid: _", channel, read_time, update_time FROM message_read_marks WHERE uid = $1"#,
            uid as UserId
        ).fetch_all(executor).await
    }

    /// Inserts the mark, or replaces the read time of the one already there.
    pub async fn upsert<'e>(executor: impl PgExecutor<'e>, value: &MessageReadMark) -> sqlx::Result<()> {
        sqlx::query!(
            "INSERT INTO message_read_marks(uid, channel, read_time, update_time) VALUES($1, $2, $3, $4)
            ON CONFLICT (uid, channel) DO UPDATE SET read_time = EXCLUDED.read_time, update_time = EXCLUDED.update_time",
            value.uid as UserId, value.channel, value.read_time, value.update_time
        ).execute(executor).await?;
        Ok(())
    }
}
