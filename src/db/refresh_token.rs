use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct RefreshToken {
    pub id: i64,
    pub user_id: i64,
    pub token_id: String,
    pub token_value: String,
    pub expires_time: DateTime<Utc>,
    pub create_time: DateTime<Utc>,
    pub last_used_time: Option<DateTime<Utc>>,
    pub device_info: Option<String>,
    pub ip_address: Option<String>,
    pub is_revoked: bool,
    pub user_agent: Option<String>,
}

pub trait IRefreshTokenDao<'e, E>: CrudDao<'e, E> 
where E: PgExecutor<'e>{
    async fn get_by_token_id(executor: E, token_id: &str) -> sqlx::Result<Option<RefreshToken>>;
    async fn list_by_uid(executor: E, uid: i64) -> sqlx::Result<Vec<RefreshToken>>;
    async fn delete_all_by_uid(executor: E, uid: i64) -> sqlx::Result<u64>;
}

pub struct RefreshTokenDao;

impl <'e, E> CrudDao<'e, E> for RefreshTokenDao 
where E: PgExecutor<'e> {
    type Entity = RefreshToken;

    async fn list(executor: E) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn page(executor: E, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(executor: E, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        sqlx::query_as!(
            Self::Entity,
            "SELECT * FROM refresh_tokens WHERE id = $1",
            id
        )
        .fetch_optional(executor)
        .await
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE refresh_tokens SET user_id = $1, token_id = $2, token_value = $3, expires_time = $4, create_time = $5, last_used_time = $6, device_info = $7, ip_address = $8, is_revoked = $9, user_agent = $10 WHERE id = $11",
            value.user_id,
            value.token_id,
            value.token_value,
            value.expires_time,
            value.create_time,
            value.last_used_time,
            value.device_info,
            value.ip_address,
            value.is_revoked,
            value.user_agent,
            value.id
        ).execute(executor).await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> sqlx::Result<i64> {
        let r = sqlx::query!(
            "INSERT INTO refresh_tokens(user_id, token_id, token_value, expires_time, create_time, last_used_time, device_info, ip_address, is_revoked, user_agent)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
            value.user_id,
            value.token_id,
            value.token_value,
            value.expires_time,
            value.create_time,
            value.last_used_time,
            value.device_info,
            value.ip_address,
            value.is_revoked,
            value.user_agent,
        ).fetch_one(executor).await?;
        Ok(r.id)
    }

    async fn delete_by_id(executor: E, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM refresh_tokens WHERE id = $1", id)
            .execute(executor)
            .await?;
        Ok(())
    }
}

impl <'e, E> IRefreshTokenDao<'e, E> for RefreshTokenDao 
where E: PgExecutor<'e> {
    async fn get_by_token_id(executor: E, token_id: &str) -> sqlx::Result<Option<RefreshToken>> {
        sqlx::query_as!(
            RefreshToken,
            "SELECT * FROM refresh_tokens WHERE token_id = $1",
            token_id
        )
        .fetch_optional(executor)
        .await
    }
    async fn list_by_uid(executor: E, uid: i64) -> sqlx::Result<Vec<RefreshToken>> {
        sqlx::query_as!(
            RefreshToken,
            "SELECT * FROM refresh_tokens WHERE user_id = $1",
            uid
        )
        .fetch_all(executor)
        .await
    }
    async fn delete_all_by_uid(executor: E, uid: i64) -> sqlx::Result<u64> {
        let rows = sqlx::query!("DELETE FROM refresh_tokens WHERE user_id = $1", uid)
            .execute(executor)
            .await?.rows_affected();
        Ok(rows)
    }
}
