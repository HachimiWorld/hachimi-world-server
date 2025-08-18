use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SongTag {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub is_active: bool,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub struct SongTagDao {
    pool: PgPool,
}

impl SongTagDao {
    pub fn new(pool: PgPool) -> Self {
        SongTagDao { pool }
    }
}

pub trait ISongTagDao: CrudDao {
    async fn list_by_ids(&self, ids: &[i64]) -> sqlx::Result<Vec<SongTag>>;
    async fn get_by_name(&self, name: &str) -> sqlx::Result<Option<SongTag>>;
    async fn search_by_prefix(&self, prefix: &str) -> sqlx::Result<Vec<SongTag>>;
}

impl CrudDao for SongTagDao {
    type Entity = SongTag;

    async fn list(&self) -> sqlx::Result<Vec<Self::Entity>> {
        sqlx::query_as!(Self::Entity, "SELECT * FROM song_tags")
            .fetch_all(&self.pool)
            .await
    }

    async fn page(&self, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(&self, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        sqlx::query_as!(Self::Entity, "SELECT * FROM song_tags WHERE id = $1", id)
            .fetch_optional(&self.pool)
            .await
    }

    async fn update_by_id(&self, value: &Self::Entity) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE song_tags SET
                name = $1,
                description = $2,
                is_active = $3,
                create_time = $4,
                update_time = $5
            WHERE id = $6",
            value.name,
            value.description,
            value.is_active,
            value.create_time,
            value.update_time,
            value.id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn insert(&self, value: &Self::Entity) -> sqlx::Result<i64> {
        sqlx::query!(
            "INSERT INTO song_tags (
                name,
                description,
                is_active,
                create_time,
                update_time
            ) VALUES ($1, $2, $3, $4, $5) RETURNING id",
            value.name,
            value.description,
            value.is_active,
            value.create_time,
            value.update_time,
        )
        .fetch_one(&self.pool)
        .await
        .map(|x| x.id)
    }

    async fn delete_by_id(&self, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM song_tags WHERE id = $1", id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

impl ISongTagDao for SongTagDao {
    async fn list_by_ids(&self, ids: &[i64]) -> sqlx::Result<Vec<SongTag>> {
        sqlx::query_as!(
            SongTag,
            "SELECT * FROM song_tags WHERE id = ANY($1)",
            ids
        ).fetch_all(&self.pool).await
    }
    
    async fn get_by_name(&self, name: &str) -> sqlx::Result<Option<SongTag>> {
        sqlx::query_as!(SongTag, "SELECT * FROM song_tags WHERE name = $1", name)
            .fetch_optional(&self.pool)
            .await
    }

    async fn search_by_prefix(&self, prefix: &str) -> sqlx::Result<Vec<SongTag>> {
        let mut escaped = prefix.replace("%", "\\%")
            .replace("_", "\\_");
        escaped.push_str("%");
        sqlx::query_as!(SongTag, "SELECT * FROM song_tags WHERE name LIKE $1 LIMIT 20", escaped)
            .fetch_all(&self.pool)
            .await
    }
}
