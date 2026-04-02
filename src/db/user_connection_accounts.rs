use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct UserConnectionAccount {
    pub user_id: i64,
    pub provider_type: String,
    pub provider_account_id: String,
    pub provider_account_name: String,
    pub public: bool,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>
}

pub struct UserConnectionAccountDao;

pub trait IUserConnectionAccountDao <'e, E>
where E: PgExecutor<'e> {
    fn insert(executor: E, value: &UserConnectionAccount) -> impl Future<Output = sqlx::Result<()>> + Send;
    fn update(executor: E, value: &UserConnectionAccount) -> impl Future<Output = sqlx::Result<()>> + Send;
    fn delete(executor: E, user_id: i64, provider_type: &str) -> impl Future<Output = sqlx::Result<()>> + Send;
    fn list_by_user_id(executor: E, user_id: i64) -> impl Future<Output = sqlx::Result<Vec<UserConnectionAccount>>> + Send;
    fn list_public_by_user_id(executor: E, user_id: i64) -> impl Future<Output = sqlx::Result<Vec<UserConnectionAccount>>> + Send;
    fn get_by_user_id(executor: E, user_id: i64, provider_type: &str) -> impl Future<Output = sqlx::Result<Option<UserConnectionAccount>>> + Send;
}

impl<'e, E> IUserConnectionAccountDao<'e, E> for UserConnectionAccountDao
where E: PgExecutor<'e> {
    async fn insert(executor: E, value: &UserConnectionAccount) -> sqlx::Result<()> {
        sqlx::query!(
            "INSERT INTO user_connection_accounts(user_id, provider_type, provider_account_id, provider_account_name, public, create_time, update_time) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            value.user_id,
            value.provider_type,
            value.provider_account_id,
            value.provider_account_name,
            value.public,
            value.create_time,
            value.update_time
        )
        .execute(executor)
        .await?;
        Ok(())
    }
    async fn update(executor: E, value: &UserConnectionAccount) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE user_connection_accounts SET provider_account_id = $1, provider_account_name = $2, public = $3, update_time = $4 WHERE user_id = $5 AND provider_type = $6",
            value.provider_account_id,
            value.provider_account_name,
            value.public,
            value.update_time,
            value.user_id,
            value.provider_type
        )
        .execute(executor)
        .await?;
        Ok(())
    }

    async fn delete(executor: E, user_id: i64, provider_type: &str) -> sqlx::Result<()> {
        sqlx::query!(
            "DELETE FROM user_connection_accounts WHERE user_id = $1 AND provider_type = $2",
            user_id,
            provider_type
        )
        .execute(executor)
        .await?;
        Ok(())
    }

    async fn list_by_user_id(executor: E, user_id: i64) -> sqlx::Result<Vec<UserConnectionAccount>> {
        sqlx::query_as!(
            UserConnectionAccount,
            "SELECT * FROM user_connection_accounts WHERE user_id = $1",
            user_id
        )
        .fetch_all(executor)
        .await
    }

    async fn list_public_by_user_id(executor: E, user_id: i64) -> sqlx::Result<Vec<UserConnectionAccount>> {
        sqlx::query_as!(
            UserConnectionAccount,
            "SELECT * FROM user_connection_accounts WHERE user_id = $1 AND public = true",
            user_id
        )
        .fetch_all(executor)
        .await
    }

    async fn get_by_user_id(executor: E, user_id: i64, provider_type: &str) -> sqlx::Result<Option<UserConnectionAccount>> {
        sqlx::query_as!(
            UserConnectionAccount,
            "SELECT * FROM user_connection_accounts WHERE user_id = $1 AND provider_type = $2",
            user_id,
            provider_type
        )
        .fetch_optional(executor)
        .await
    }
}