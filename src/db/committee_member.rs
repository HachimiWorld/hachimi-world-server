use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

macro_rules! query_committee_members {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            CommitteeMember, r#"
            SELECT
                uid,
                appointed_by_uid,
                create_time
            FROM committee_members
            "# + $extra,
            $($arg),*
        )
    };
}

/// @since 261008
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitteeMember {
    pub uid: i64,
    pub appointed_by_uid: i64,
    pub create_time: DateTime<Utc>,
}

pub struct CommitteeMemberDao;

impl CommitteeMemberDao {
    pub async fn get<'e>(executor: impl PgExecutor<'e>, uid: i64) -> sqlx::Result<Option<CommitteeMember>> {
        query_committee_members!("WHERE uid = $1", uid)
            .fetch_optional(executor).await
    }

    /// Oldest appointment first.
    pub async fn list_all<'e>(executor: impl PgExecutor<'e>) -> sqlx::Result<Vec<CommitteeMember>> {
        query_committee_members!("ORDER BY create_time, uid")
            .fetch_all(executor).await
    }

    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &CommitteeMember) -> sqlx::Result<()> {
        sqlx::query!(
            "INSERT INTO committee_members (uid, appointed_by_uid, create_time) VALUES ($1, $2, $3)",
            value.uid, value.appointed_by_uid, value.create_time
        ).execute(executor).await?;
        Ok(())
    }

    /// Returns whether a member was removed.
    pub async fn delete<'e>(executor: impl PgExecutor<'e>, uid: i64) -> sqlx::Result<bool> {
        sqlx::query!("DELETE FROM committee_members WHERE uid = $1", uid)
            .execute(executor).await.map(|x| x.rows_affected() > 0)
    }
}
