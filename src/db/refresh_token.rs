use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

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
}

pub trait IRefreshTokenDao: CrudDao<Entity = RefreshToken> {
    async fn get_by_token_id(&self, token_id: &str) -> sqlx::Result<Option<RefreshToken>>;
    async fn list_by_uid(&self, uid: i64) -> sqlx::Result<Vec<RefreshToken>>;
    async fn delete_all_by_uid(&self, uid: i64) -> sqlx::Result<u64>;
}

pub struct RefreshTokenDao {
    pool: PgPool,
}

impl RefreshTokenDao {
    pub fn new(pool: PgPool) -> Self {
        RefreshTokenDao { pool }
    }
}

impl CrudDao for RefreshTokenDao {
    type Entity = RefreshToken;

    async fn list(&self) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn page(&self, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(&self, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        sqlx::query_as!(
            Self::Entity,
            "SELECT * FROM refresh_tokens WHERE id = $1",
            id
        )
        .fetch_optional(&self.pool)
        .await
    }

    async fn update_by_id(&self, value: &Self::Entity) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE refresh_tokens SET user_id = $1, token_id = $2, token_value = $3, expires_time = $4, create_time = $5, last_used_time = $6, device_info = $7, ip_address = $8, is_revoked = $9 WHERE id = $10",
            value.user_id,
            value.token_id,
            value.token_value,
            value.expires_time,
            value.create_time,
            value.last_used_time,
            value.device_info,
            value.ip_address,
            value.is_revoked,
            value.id
        ).execute(&self.pool).await?;
        Ok(())
    }

    async fn insert(&self, value: &Self::Entity) -> sqlx::Result<i64> {
        let r = sqlx::query!(
            "INSERT INTO refresh_tokens(user_id, token_id, token_value, expires_time, create_time, last_used_time, device_info, ip_address, is_revoked)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
            value.user_id,
            value.token_id,
            value.token_value,
            value.expires_time,
            value.create_time,
            value.last_used_time,
            value.device_info,
            value.ip_address,
            value.is_revoked
        ).fetch_one(&self.pool).await?;
        Ok(r.id)
    }

    async fn delete_by_id(&self, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM refresh_tokens WHERE id = $1", id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

impl IRefreshTokenDao for RefreshTokenDao {
    async fn get_by_token_id(&self, token_id: &str) -> sqlx::Result<Option<RefreshToken>> {
        sqlx::query_as!(
            RefreshToken,
            "SELECT * FROM refresh_tokens WHERE token_id = $1",
            token_id
        )
        .fetch_optional(&self.pool)
        .await
    }
    async fn list_by_uid(&self, uid: i64) -> sqlx::Result<Vec<RefreshToken>> {
        sqlx::query_as!(
            RefreshToken,
            "SELECT * FROM refresh_tokens WHERE user_id = $1",
            uid
        )
        .fetch_all(&self.pool)
        .await
    }
    async fn delete_all_by_uid(&self, uid: i64) -> sqlx::Result<u64> {
        let rows = sqlx::query!("DELETE FROM refresh_tokens WHERE user_id = $1", uid)
            .execute(&self.pool)
            .await?.rows_affected();
        Ok(rows)
    }
}
