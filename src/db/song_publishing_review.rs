use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{query, query_as, FromRow, PgExecutor};
use crate::db::CrudDao;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SongPublishingReview {
    pub id: i64,
    pub user_id: i64,
    pub song_display_id: String,
    pub data: Value,
    pub submit_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
    pub review_time: Option<DateTime<Utc>>,
    pub review_comment: Option<String>,
    pub status: i32,
}

pub struct SongPublishingReviewDao;

pub trait ISongPublishingReviewDao<'e, E>: CrudDao<'e, E>
where
    E: PgExecutor<'e>,
{
    async fn count(executor: E) -> sqlx::Result<i64>;
    async fn page_by_user(executor: E, user_id: i64, page_size: i64, page_index: i64) -> sqlx::Result<Vec<Self::Entity>>;
    async fn count_by_user(executor: E, user_id: i64) -> sqlx::Result<i64>;
}

impl<'e, E> CrudDao<'e, E> for SongPublishingReviewDao
where
    E: PgExecutor<'e>,
{
    type Entity = SongPublishingReview;

    async fn list(executor: E) -> sqlx::Result<Vec<Self::Entity>> {
        query_as!(Self::Entity, "SELECT * FROM song_publishing_review")
            .fetch_all(executor).await
    }

    async fn page(executor: E, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        query_as!(Self::Entity, "SELECT * FROM song_publishing_review ORDER BY id DESC LIMIT $1 OFFSET $2", size, page * size)
            .fetch_all(executor).await
    }

    async fn get_by_id(executor: E, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        query_as!(Self::Entity, "SELECT * FROM song_publishing_review WHERE id = $1", id)
            .fetch_optional(executor).await
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> sqlx::Result<()> {
        query!(
            "UPDATE song_publishing_review SET
                user_id = $1,
                song_display_id = $2,
                data = $3,
                submit_time = $4,
                update_time = $5,
                review_time = $6,
                review_comment = $7,
                status = $8
            WHERE id = $9",
            value.user_id,
            value.song_display_id,
            value.data,
            value.submit_time,
            value.update_time,
            value.review_time,
            value.review_comment,
            value.status,
            value.id,
        ).execute(executor).await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> sqlx::Result<i64> {
        query!("INSERT INTO song_publishing_review (user_id, song_display_id, data, submit_time, update_time, review_time, review_comment, status)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                RETURNING id",
            value.user_id, value.song_display_id, value.data, value.submit_time, value.update_time, value.review_time, value.review_comment, value.status
        ).fetch_one(executor).await.map(|r| r.id)
    }

    async fn delete_by_id(executor: E, id: i64) -> sqlx::Result<()> {
        query!("DELETE FROM song_publishing_review WHERE id = $1", id).execute(executor).await?;
        Ok(())
    }
}

impl<'e, E> ISongPublishingReviewDao<'e, E> for SongPublishingReviewDao
where
    E: PgExecutor<'e>,
{
    async fn count(executor: E) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(*) FROM song_publishing_review")
            .fetch_one(executor).await
            .map(|r| r.count.unwrap_or(0))
    }

    async fn page_by_user(executor: E, user_id: i64, page_index: i64, page_size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        sqlx::query_as!(
            Self::Entity,
            "SELECT * FROM song_publishing_review WHERE user_id = $1 ORDER BY id DESC LIMIT $2 OFFSET $3",
            user_id, page_size, page_index * page_size
        ).fetch_all(executor).await
    }

    async fn count_by_user(executor: E, user_id: i64) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(*) FROM song_publishing_review WHERE user_id = $1", user_id)
            .fetch_one(executor).await
            .map(|r| r.count.unwrap_or(0))
    }
}