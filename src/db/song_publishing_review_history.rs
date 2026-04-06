use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgExecutor, Result};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SongPublishingReviewHistory {
    pub id: i64,
    pub review_id: i64,
    pub user_id: i64,
    pub action_type: i32,
    pub note: Option<String>,
    pub snapshot_data: Value,
    pub create_time: DateTime<Utc>,
}

pub const ACTION_SUBMIT: i32 = 0;
pub const ACTION_MODIFY: i32 = 1;

pub struct SongPublishingReviewHistoryDao;

pub trait ISongPublishingReviewHistoryDao<'e, E>: CrudDao<'e, E>
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

impl<'e, E> CrudDao<'e, E> for SongPublishingReviewHistoryDao
where
    E: PgExecutor<'e>,
{
    type Entity = SongPublishingReviewHistory;

    async fn list(executor: E) -> Result<Vec<Self::Entity>> {
        sqlx::query_as!(
            Self::Entity,
            "SELECT * FROM song_publishing_review_history ORDER BY create_time DESC, id DESC",
        )
        .fetch_all(executor)
        .await
    }

    async fn page(executor: E, page_index: i64, page_size: i64) -> Result<Vec<Self::Entity>> {
        sqlx::query_as!(
            Self::Entity,
            "SELECT * FROM song_publishing_review_history ORDER BY create_time DESC, id DESC LIMIT $1 OFFSET $2",
            page_size,
            page_index * page_size
        )
        .fetch_all(executor)
        .await
    }

    async fn get_by_id(executor: E, id: i64) -> Result<Option<Self::Entity>> {
        sqlx::query_as!(
            Self::Entity,
            "SELECT * FROM song_publishing_review_history WHERE id = $1",
            id
        )
        .fetch_optional(executor)
        .await
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> Result<()> {
        sqlx::query!(
            "UPDATE song_publishing_review_history SET review_id = $1, user_id = $2, action_type = $3, note = $4, snapshot_data = $5, create_time = $6 WHERE id = $7",
            value.review_id,
            value.user_id,
            value.action_type,
            value.note,
            value.snapshot_data,
            value.create_time,
            value.id
        )
        .execute(executor)
        .await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> Result<i64> {
        sqlx::query_scalar!(
            "INSERT INTO song_publishing_review_history (review_id, user_id, action_type, note, snapshot_data, create_time) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
            value.review_id,
            value.user_id,
            value.action_type,
            value.note,
            value.snapshot_data,
            value.create_time
        )
        .fetch_one(executor)
        .await
    }

    async fn delete_by_id(executor: E, id: i64) -> Result<()> {
        sqlx::query!(
            "DELETE FROM song_publishing_review_history WHERE id = $1",
            id
        )
        .execute(executor)
        .await?;
        Ok(())
    }
}

impl<'e, E> ISongPublishingReviewHistoryDao<'e, E> for SongPublishingReviewHistoryDao
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
            "SELECT * FROM song_publishing_review_history WHERE review_id = $1 ORDER BY create_time DESC, id DESC LIMIT $2 OFFSET $3",
            review_id,
            page_size,
            page_index * page_size
        )
        .fetch_all(executor)
        .await
    }

    async fn count_by_review_id(executor: E, review_id: i64) -> Result<i64> {
        sqlx::query_scalar!(
            "SELECT COUNT(*) FROM song_publishing_review_history WHERE review_id = $1",
            review_id
        )
        .bind(review_id)
        .fetch_one(executor)
        .await
        .map(|count| count.unwrap_or(0))
    }
}

