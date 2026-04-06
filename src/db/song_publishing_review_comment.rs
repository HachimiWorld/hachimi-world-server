use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, Result};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SongPublishingReviewComment {
    pub id: i64,
    pub review_id: i64,
    pub user_id: i64,
    pub content: String,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub struct SongPublishingReviewCommentDao;

pub trait ISongPublishingReviewCommentDao<'e, E>: CrudDao<'e, E>
where
    E: PgExecutor<'e>,
{
    fn page_by_review_id(
        executor: E,
        review_id: i64,
        page_index: i64,
        page_size: i64,
    ) -> impl Future<Output = Result<Vec<Self::Entity>>> + Send;

    fn count_by_review_id(executor: E, review_id: i64)
        -> impl Future<Output = Result<i64>> + Send;
}

impl<'e, E> CrudDao<'e, E> for SongPublishingReviewCommentDao
where
    E: PgExecutor<'e>,
{
    type Entity = SongPublishingReviewComment;

    async fn list(executor: E) -> Result<Vec<Self::Entity>> {
        sqlx::query_as!(
            Self::Entity,
            "SELECT * FROM song_publishing_review_comment ORDER BY create_time"
        ).fetch_all(executor).await
    }

    async fn page(executor: E, page_index: i64, page_size: i64) -> Result<Vec<Self::Entity>> {
        sqlx::query_as!(
            Self::Entity,
            "SELECT * FROM song_publishing_review_comment ORDER BY create_time DESC LIMIT $1 OFFSET $2",
            page_size, page_index * page_size
        )
        .fetch_all(executor)
        .await
    }

    async fn get_by_id(executor: E, id: i64) -> Result<Option<Self::Entity>> {
        sqlx::query_as!(
            Self::Entity,
            "SELECT * FROM song_publishing_review_comment WHERE id = $1",
            id
        ).fetch_optional(executor).await
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> Result<()> {
        sqlx::query!(
            "UPDATE song_publishing_review_comment SET review_id = $1, user_id = $2, content = $3, create_time = $4, update_time = $5 WHERE id = $6",
            value.review_id,
            value.user_id,
            &value.content,
            value.create_time,
            value.update_time,
            value.id
        )
        .execute(executor)
        .await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> Result<i64> {
        let record = sqlx::query_scalar!(
            "INSERT INTO song_publishing_review_comment (review_id, user_id, content, create_time, update_time) VALUES ($1, $2, $3, $4, $5) RETURNING id",
            value.review_id,
            value.user_id,
            &value.content,
            value.create_time,
            value.update_time
        )
        .fetch_one(executor)
        .await?;
        Ok(record)
    }

    async fn delete_by_id(executor: E, id: i64) -> Result<()> {
        sqlx::query!(
            "DELETE FROM song_publishing_review_comment WHERE id = $1",
            id
        ).execute(executor).await?;
        Ok(())
    }
}

impl<'e, E> ISongPublishingReviewCommentDao<'e, E> for SongPublishingReviewCommentDao
where
    E: PgExecutor<'e>,
{
    async fn page_by_review_id(
        executor: E,
        review_id: i64,
        page_index: i64,
        page_size: i64,
    ) -> Result<Vec<Self::Entity>> {
        sqlx::query_as!(
            Self::Entity,
            "SELECT * FROM song_publishing_review_comment WHERE review_id = $1 ORDER BY create_time ASC LIMIT $2 OFFSET $3",
            review_id,
            page_size,
            page_index * page_size
        )
        .fetch_all(executor)
        .await
    }

    async fn count_by_review_id(executor: E, review_id: i64) -> Result<i64> {
        sqlx::query_scalar!(
            "SELECT COUNT(*) FROM song_publishing_review_comment WHERE review_id = $1",
            review_id
        )
        .fetch_one(executor)
        .await
        .map(|count| count.unwrap_or(0))
    }
}


