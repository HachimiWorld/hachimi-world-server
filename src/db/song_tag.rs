use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SongTag {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub is_active: bool,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub struct SongTagDao;

pub trait ISongTagDao<'e, E>: CrudDao<'e, E>
where E: PgExecutor<'e> {
    async fn list_by_ids(executor: E, ids: &[i64]) -> sqlx::Result<Vec<SongTag>>;
    async fn get_by_name(executor: E, name: &str) -> sqlx::Result<Option<SongTag>>;
    async fn search_by_prefix(executor: E, prefix: &str) -> sqlx::Result<Vec<SongTag>>;
}

impl <'e, E> CrudDao<'e, E> for SongTagDao 
where E: PgExecutor<'e> {
    type Entity = SongTag;

    async fn list(executor: E) -> sqlx::Result<Vec<Self::Entity>> {
        sqlx::query_as!(Self::Entity, "SELECT * FROM song_tags")
            .fetch_all(executor)
            .await
    }

    async fn page(executor: E, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(executor: E, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        sqlx::query_as!(Self::Entity, "SELECT * FROM song_tags WHERE id = $1", id)
            .fetch_optional(executor)
            .await
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> sqlx::Result<()> {
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
        .execute(executor)
        .await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> sqlx::Result<i64> {
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
        .fetch_one(executor)
        .await
        .map(|x| x.id)
    }

    async fn delete_by_id(executor: E, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM song_tags WHERE id = $1", id)
            .execute(executor)
            .await?;
        Ok(())
    }
}

impl <'e, E> ISongTagDao<'e, E> for SongTagDao 
where E: PgExecutor<'e> {
    async fn list_by_ids(executor: E, ids: &[i64]) -> sqlx::Result<Vec<SongTag>> {
        sqlx::query_as!(
            SongTag,
            "SELECT * FROM song_tags WHERE id = ANY($1)",
            ids
        ).fetch_all(executor).await
    }
    
    async fn get_by_name(executor: E, name: &str) -> sqlx::Result<Option<SongTag>> {
        sqlx::query_as!(SongTag, "SELECT * FROM song_tags WHERE name = $1", name)
            .fetch_optional(executor)
            .await
    }

    async fn search_by_prefix(executor: E, prefix: &str) -> sqlx::Result<Vec<SongTag>> {
        let mut escaped = prefix.replace("%", "\\%")
            .replace("_", "\\_");
        escaped.push_str("%");
        sqlx::query_as!(SongTag, "SELECT * FROM song_tags WHERE name LIKE $1 LIMIT 20", escaped)
            .fetch_all(executor)
            .await
    }
}
