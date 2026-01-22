use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

#[derive(sqlx::FromRow)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    pub id: i64,
    pub version_name: String,
    pub version_number: i32,
    pub changelog: String,
    pub variant: String,
    pub url: String,
    pub release_time: DateTime<Utc>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub struct VersionDao;

impl<'e, E> CrudDao<'e, E> for VersionDao
where
    E: PgExecutor<'e>,
{
    type Entity = Version;

    async fn list(_executor: E) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn page(_executor: E, _page: i64, _size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(executor: E, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        sqlx::query_as!(Self::Entity, "SELECT * FROM version WHERE id = $1", id)
            .fetch_optional(executor).await
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> sqlx::Result<()> {
        sqlx::query!("
            UPDATE version SET 
                version_name = $1,
                version_number = $2,
                changelog = $3,
                variant = $4,
                url = $5,
                release_time = $6,
                update_time = $7
            WHERE id = $8",
            value.version_name, 
            value.version_number, 
            value.changelog, 
            value.variant, 
            value.url, 
            value.release_time, 
            value.update_time, 
            value.id
        ).execute(executor).await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> sqlx::Result<i64> {
        sqlx::query!("INSERT INTO version(
                version_name,
                version_number,
                changelog,
                variant,
                url,
                release_time,
                create_time,
                update_time
            ) VALUES($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
            value.version_name,
            value.version_number,
            value.changelog,
            value.variant,
            value.url,
            value.release_time,
            value.create_time,
            value.update_time
        ).fetch_one(executor).await.map(|x| x.id)
    }

    async fn delete_by_id(executor: E, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM version WHERE id = $1", id).execute(executor).await?;
        Ok(())
    }
}

impl<'e> VersionDao {
    pub async fn get_latest_version(executor: impl PgExecutor<'e>, variant: &str, end_time: DateTime<Utc>) -> sqlx::Result<Option<Version>> {
        sqlx::query_as!(Version, "SELECT * FROM version WHERE variant = $1 AND release_time <= $2 ORDER BY release_time DESC LIMIT 1", variant, end_time)
            .fetch_optional(executor)
            .await
    }
}