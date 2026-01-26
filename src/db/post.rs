use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, Result};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Post {
    pub id: i64,
    pub author_uid: i64,
    pub title: String,
    pub content: String,
    pub content_type: String,
    pub cover_url: Option<String>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub struct PostDao;

impl<'e, E> CrudDao<'e, E> for PostDao
where
    E: PgExecutor<'e>,
{
    type Entity = Post;

    async fn list(executor: E) -> Result<Vec<Self::Entity>> {
        sqlx::query_as!(Self::Entity, "SELECT * FROM posts")
            .fetch_all(executor)
            .await
    }

    async fn page(executor: E, page_index: i64, page_size: i64) -> Result<Vec<Self::Entity>> {
        Ok(sqlx::query_as!(Self::Entity, "SELECT * FROM posts ORDER BY create_time DESC LIMIT $1 OFFSET $2", page_size, page_index * page_size)
            .fetch_all(executor)
            .await?)
    }

    async fn get_by_id(executor: E, id: i64) -> Result<Option<Self::Entity>> {
        Ok(sqlx::query_as!(Self::Entity, "SELECT * FROM posts WHERE id = $1", id)
            .fetch_optional(executor)
            .await?)
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> Result<()> {
        // Update fields and set update_time
        sqlx::query!(
            "UPDATE posts SET title = $1, content = $2, content_type = $3, cover_url = $4, update_time = $5 WHERE id = $6",
            value.title,
            value.content,
            value.content_type,
            value.cover_url,
            value.update_time,
            value.id
        )
        .execute(executor)
        .await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> Result<i64> {
        // Insert a post, returning the generated id
        let rec = sqlx::query!(
            "INSERT INTO posts (author_uid, title, content, content_type, cover_url, create_time, update_time)
            VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
            value.author_uid,
            value.title,
            value.content,
            value.content_type,
            value.cover_url,
            value.create_time,
            value.update_time
        )
        .fetch_one(executor)
        .await?;

        Ok(rec.id)
    }

    async fn delete_by_id(executor: E, id: i64) -> Result<()> {
        sqlx::query!("DELETE FROM posts WHERE id = $1", id)
            .execute(executor)
            .await?;
        Ok(())
    }
}