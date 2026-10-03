use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

macro_rules! query_sitemap_generations {
    () => {
        sqlx::query_as!(
            SitemapGeneration, r#"
            SELECT
                id,
                trigger_type,
                status,
                song_count,
                file_count,
                error,
                start_time,
                finish_time
            FROM sitemap_generations
            "#
        )
    };
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            SitemapGeneration, r#"
            SELECT
                id,
                trigger_type,
                status,
                song_count,
                file_count,
                error,
                start_time,
                finish_time
            FROM sitemap_generations
            "# + $extra,
            $($arg),*
        )
    };
}

/// @since 261003
#[derive(sqlx::FromRow)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SitemapGeneration {
    pub id: i64,
    /// `schedule` or `manual`.
    pub trigger_type: String,
    /// `running`, `success` or `failure`.
    pub status: String,
    pub song_count: Option<i32>,
    pub file_count: Option<i32>,
    pub error: Option<String>,
    pub start_time: DateTime<Utc>,
    pub finish_time: Option<DateTime<Utc>>,
}

pub struct SitemapGenerationDao;

impl<'e, E> CrudDao<'e, E> for SitemapGenerationDao
where
    E: PgExecutor<'e>,
{
    type Entity = SitemapGeneration;

    async fn list(executor: E) -> sqlx::Result<Vec<Self::Entity>> {
        query_sitemap_generations!("ORDER BY start_time DESC, id DESC")
            .fetch_all(executor)
            .await
    }

    /// Most recent first.
    async fn page(executor: E, page_index: i64, page_size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        query_sitemap_generations!("ORDER BY start_time DESC, id DESC LIMIT $1 OFFSET $2", page_size, page_index * page_size)
            .fetch_all(executor)
            .await
    }

    async fn get_by_id(executor: E, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        query_sitemap_generations!("WHERE id = $1", id)
            .fetch_optional(executor)
            .await
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> sqlx::Result<()> {
        sqlx::query!("
            UPDATE sitemap_generations SET
                trigger_type = $1,
                status = $2,
                song_count = $3,
                file_count = $4,
                error = $5,
                start_time = $6,
                finish_time = $7
            WHERE id = $8",
            value.trigger_type,
            value.status,
            value.song_count,
            value.file_count,
            value.error,
            value.start_time,
            value.finish_time,
            value.id
        ).execute(executor).await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> sqlx::Result<i64> {
        sqlx::query!("INSERT INTO sitemap_generations(
                trigger_type,
                status,
                song_count,
                file_count,
                error,
                start_time,
                finish_time
            ) VALUES($1, $2, $3, $4, $5, $6, $7) RETURNING id",
            value.trigger_type,
            value.status,
            value.song_count,
            value.file_count,
            value.error,
            value.start_time,
            value.finish_time
        ).fetch_one(executor).await.map(|x| x.id)
    }

    async fn delete_by_id(executor: E, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM sitemap_generations WHERE id = $1", id).execute(executor).await?;
        Ok(())
    }
}

impl<'e> SitemapGenerationDao {
    pub async fn count(executor: impl PgExecutor<'e>) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(*) FROM sitemap_generations")
            .fetch_one(executor)
            .await
            .map(|x| x.count.unwrap_or(0))
    }
}
