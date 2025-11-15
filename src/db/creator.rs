use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Creator {
    pub id: i64,
    pub user_id: i64,
    pub jmid_prefix: String,
    pub active: bool,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub struct CreatorDao;

impl<'e, E> CrudDao<'e, E> for CreatorDao
where
    E: PgExecutor<'e>,
{
    type Entity = Creator;

    async fn list(executor: E) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn page(executor: E, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(executor: E, id: i64) -> sqlx::Result<Option<Creator>> {
        sqlx::query_as!(Creator, "SELECT * FROM creators WHERE id = $1", id)
            .fetch_optional(executor)
            .await
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE creators SET
                user_id = $1,
                jmid_prefix = $2,
                active = $3,
                create_time = $4,
                update_time = $5
            WHERE id = $6",
            value.user_id,
            value.jmid_prefix,
            value.active,
            value.create_time,
            value.update_time,
            value.id,
        ).execute(executor).await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> sqlx::Result<i64> {
        sqlx::query!(
            "INSERT INTO creators (user_id, jmid_prefix, active, create_time, update_time) VALUES ($1, $2, $3, $4, $5) RETURNING id",
            value.user_id,
            value.jmid_prefix,
            value.active,
            value.create_time,
            value.update_time
        ).fetch_one(executor).await
            .map(|x| x.id)
    }

    async fn delete_by_id(executor: E, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM creators WHERE id = $1", id)
            .execute(executor)
            .await?;
        Ok(())
    }
}

impl<'e> CreatorDao {

    pub async fn get_by_user_id(executor: impl PgExecutor<'e>, user_id: i64) -> sqlx::Result<Option<Creator>> {
        sqlx::query_as!(Creator, "SELECT * FROM creators WHERE user_id = $1", user_id)
            .fetch_optional(executor)
            .await
    }

    pub async fn get_by_jmid_prefix(executor: impl PgExecutor<'e>, jmid_prefix: &str) -> sqlx::Result<Option<Creator>> {
        sqlx::query_as!(Creator, "SELECT * FROM creators WHERE jmid_prefix = $1", jmid_prefix)
            .fetch_optional(executor)
            .await
    }
}