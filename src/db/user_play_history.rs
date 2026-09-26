use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, PgTransaction};

macro_rules! query_user_play_history {
    () => {
        sqlx::query_as!(
            UserPlayHistory, r#"
            SELECT
                user_id,
                song_id,
                create_time
            FROM user_play_history
            "#
        )
    };
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            UserPlayHistory, r#"
            SELECT
                user_id,
                song_id,
                create_time
            FROM user_play_history
            "# + $extra,
            $($arg),*
        )
    };
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct UserPlayHistory {
    pub user_id: i64,
    pub song_id: i64,
    pub create_time: DateTime<Utc>
}

pub struct UserPlayHistoryDao;

pub trait IUserPlayHistory<'e, E>
where
    E: PgExecutor<'e>, {
    fn cursor_by_user_id(executor: E, user_id: i64, before_time: DateTime<Utc>, size: usize) -> impl Future<Output = sqlx::Result<Vec<UserPlayHistory>>>;
}

pub trait IUserPlayHistoryExt<'e> {
    fn delete_and_insert(executor: &mut PgTransaction<'e>, user_id: i64, song_id: i64) -> impl Future<Output = sqlx::Result<()>>;
}

impl<'e, E> IUserPlayHistory<'e, E> for UserPlayHistoryDao
where
    E: PgExecutor<'e> {
    async fn cursor_by_user_id(executor: E, user_id: i64, before_time: DateTime<Utc>, size: usize) -> sqlx::Result<Vec<UserPlayHistory>> {
        query_user_play_history!(
            "WHERE user_id = $1 AND create_time < $2 ORDER BY create_time DESC LIMIT $3",
            user_id,
            before_time,
            size as i64
        ).fetch_all(executor).await
    }
}

impl<'e> IUserPlayHistoryExt<'e> for UserPlayHistoryDao {
    async fn delete_and_insert(executor: &mut PgTransaction<'e>, user_id: i64, song_id: i64) -> sqlx::Result<()> {
        sqlx::query!(
            "DELETE FROM user_play_history WHERE user_id = $1 AND song_id = $2",
            user_id,
            song_id
        ).execute(&mut **executor).await?;
        sqlx::query!(
            "INSERT INTO user_play_history (user_id, song_id, create_time) VALUES ($1, $2, $3)",
            user_id,
            song_id,
            Utc::now()
        ).execute(&mut **executor).await?;
        Ok(())
    }
}