use crate::db::song::SongId;
use chrono::{DateTime, Utc};
use crate::db::user::UserId;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, PgTransaction};

macro_rules! query_user_play_history {
    () => {
        sqlx::query_as!(
            UserPlayHistory, r#"
            SELECT
                user_id as "user_id: _",
                song_id as "song_id: _",
                create_time
            FROM user_play_history
            "#
        )
    };
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            UserPlayHistory, r#"
            SELECT
                user_id as "user_id: _",
                song_id as "song_id: _",
                create_time
            FROM user_play_history
            "# + $extra,
            $($arg),*
        )
    };
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct UserPlayHistory {
    pub user_id: UserId,
    pub song_id: SongId,
    pub create_time: DateTime<Utc>
}

pub struct UserPlayHistoryDao;

pub trait IUserPlayHistory<'e, E>
where
    E: PgExecutor<'e>, {
    fn cursor_by_user_id(executor: E, user_id: UserId, before_time: DateTime<Utc>, size: usize) -> impl Future<Output = sqlx::Result<Vec<UserPlayHistory>>>;
    fn delete(executor: E, user_id: UserId, song_id: SongId) -> impl Future<Output = sqlx::Result<()>>;
}

pub trait IUserPlayHistoryExt<'e> {
    fn delete_and_insert(executor: &mut PgTransaction<'e>, user_id: UserId, song_id: SongId) -> impl Future<Output = sqlx::Result<()>>;
}

impl<'e, E> IUserPlayHistory<'e, E> for UserPlayHistoryDao
where
    E: PgExecutor<'e> {
    async fn cursor_by_user_id(executor: E, user_id: UserId, before_time: DateTime<Utc>, size: usize) -> sqlx::Result<Vec<UserPlayHistory>> {
        query_user_play_history!(
            "WHERE user_id = $1 AND create_time < $2 ORDER BY create_time DESC LIMIT $3",
            user_id as UserId,
            before_time,
            size as i64
        ).fetch_all(executor).await
    }

    async fn delete(executor: E, user_id: UserId, song_id: SongId) -> sqlx::Result<()> {
        sqlx::query!(
            "DELETE FROM user_play_history WHERE user_id = $1 AND song_id = $2",
            user_id as UserId,
            song_id as SongId
        ).execute(executor).await?;
        Ok(())
    }
}

impl<'e> IUserPlayHistoryExt<'e> for UserPlayHistoryDao {
    async fn delete_and_insert(executor: &mut PgTransaction<'e>, user_id: UserId, song_id: SongId) -> sqlx::Result<()> {
        sqlx::query!(
            "DELETE FROM user_play_history WHERE user_id = $1 AND song_id = $2",
            user_id as UserId,
            song_id as SongId
        ).execute(&mut **executor).await?;
        sqlx::query!(
            "INSERT INTO user_play_history (user_id, song_id, create_time) VALUES ($1, $2, $3)",
            user_id as UserId,
            song_id as SongId,
            Utc::now()
        ).execute(&mut **executor).await?;
        Ok(())
    }
}