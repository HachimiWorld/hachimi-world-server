use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Song {
    pub id: i64,
    pub display_id: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub artist: String,
    pub file_url: String,
    pub cover_art_url: String,
    pub lyrics: String,
    pub duration_seconds: i32,
    pub uploader_uid: i64,
    pub creation_type: i32,
    pub play_count: i64,
    pub like_count: i64,
    pub is_private: bool,
    pub release_time: DateTime<Utc>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SongOriginInfo {
    pub id: i64,
    pub song_id: i64,
    pub origin_type: i32,
    pub origin_song_id: Option<i64>,
    pub origin_title: Option<String>,
    pub origin_artist: Option<String>,
    pub origin_url: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SongProductionCrew {
    pub id: i64,
    pub song_id: i64,
    pub role: String,
    pub uid: Option<i64>,
    pub person_name: Option<String>,
}

pub struct SongDao {
    pool: PgPool,
}

impl SongDao {
    pub fn new(pool: PgPool) -> Self {
        SongDao { pool }
    }
}

pub trait ISongDao: CrudDao {
    async fn get_by_display_id(&self, display_id: &str) -> sqlx::Result<Option<Song>>;
    async fn update_song_production_crew(
        &self,
        song_id: i64,
        values: &[SongProductionCrew],
    ) -> sqlx::Result<()>;
    async fn update_song_origin_info(
        &self,
        song_id: i64,
        values: &[SongOriginInfo],
    ) -> sqlx::Result<()>;
    async fn update_song_tags(&self, song_id: i64, tags: Vec<i64>) -> sqlx::Result<()>;
    async fn list_tags_by_song_id(&self, song_id: i64) -> sqlx::Result<Vec<i64>>;
    async fn list_origin_info_by_song_id(&self, song_id: i64) -> sqlx::Result<Vec<SongOriginInfo>>;
    async fn list_production_crew_by_song_id(&self, song_id: i64) -> sqlx::Result<Vec<SongProductionCrew>>;
    async fn list_by_ids(&self, ids: &[i64]) -> sqlx::Result<Vec<Self::Entity>>;
}

impl CrudDao for SongDao {
    type Entity = Song;

    async fn list(&self) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn page(&self, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(&self, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        sqlx::query_as!(Song, "SELECT * FROM songs WHERE id = $1", id)
            .fetch_optional(&self.pool)
            .await
    }

    async fn update_by_id(&self, value: &Self::Entity) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE songs SET
                display_id = $1,
                title = $2,
                subtitle = $3,
                description = $4,
                artist = $5,
                file_url = $6,
                cover_art_url = $7,
                lyrics = $8,
                duration_seconds = $9,
                uploader_uid = $10,
                creation_type = $11,
                play_count = $12,
                like_count = $13,
                is_private = $14,
                release_time = $15,
                create_time = $16,
                update_time = $17
            WHERE id = $18",
            value.display_id,
            value.title,
            value.subtitle,
            value.description,
            value.artist,
            value.file_url,
            value.cover_art_url,
            value.lyrics,
            value.duration_seconds,
            value.uploader_uid,
            value.creation_type,
            value.play_count,
            value.like_count,
            value.is_private,
            value.release_time,
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
            "INSERT INTO songs (
                display_id,
                title,
                subtitle,
                description,
                artist,
                file_url,
                cover_art_url,
                lyrics,
                duration_seconds,
                uploader_uid,
                creation_type,
                play_count,
                like_count,
                is_private,
                release_time,
                create_time,
                update_time
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17) RETURNING id",
            value.display_id,
            value.title,
            value.subtitle,
            value.description,
            value.artist,
            value.file_url,
            value.cover_art_url,
            value.lyrics,
            value.duration_seconds,
            value.uploader_uid,
            value.creation_type,
            value.play_count,
            value.like_count,
            value.is_private,
            value.release_time,
            value.create_time,
            value.update_time
        ).fetch_one(&self.pool).await.map(|x| x.id)
    }

    async fn delete_by_id(&self, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM songs WHERE id = $1", id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

impl ISongDao for SongDao {
    async fn get_by_display_id(&self, display_id: &str) -> sqlx::Result<Option<Song>> {
        sqlx::query_as!(
            Song,
            "SELECT * FROM songs WHERE display_id = $1",
            display_id
        )
        .fetch_optional(&self.pool)
        .await
    }

    async fn update_song_production_crew(
        &self,
        song_id: i64,
        values: &[SongProductionCrew],
    ) -> sqlx::Result<()> {
        sqlx::query!(
            "DELETE FROM song_production_crew WHERE song_id = $1",
            song_id
        )
        .execute(&self.pool)
        .await?;
        for x in values {
            sqlx::query!(
                "INSERT INTO song_production_crew (
                    song_id,
                    role,
                    uid,
                    person_name
                ) VALUES ($1, $2, $3, $4)",
                x.song_id,
                x.role,
                x.uid,
                x.person_name
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn update_song_origin_info(
        &self,
        song_id: i64,
        values: &[SongOriginInfo],
    ) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM song_origin_info WHERE song_id = $1", song_id)
            .execute(&self.pool)
            .await?;
        for x in values {
            sqlx::query!(
                "INSERT INTO song_origin_info (
                    song_id,
                    origin_type,
                    origin_song_id,
                    origin_title,
                    origin_artist,
                    origin_url
                ) VALUES ($1, $2, $3, $4, $5, $6)",
                x.song_id,
                x.origin_type,
                x.origin_song_id,
                x.origin_title,
                x.origin_artist,
                x.origin_url
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn update_song_tags(&self, song_id: i64, tags: Vec<i64>) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM song_tag_refs WHERE song_id = $1", song_id)
            .execute(&self.pool)
            .await?;
        for tag_id in tags {
            sqlx::query!(
                "INSERT INTO song_tag_refs (song_id, tag_id) VALUES ($1, $2)",
                song_id, tag_id
            ).execute(&self.pool).await?;
        }
        Ok(())
    }

    async fn list_tags_by_song_id(&self, song_id: i64) -> sqlx::Result<Vec<i64>> {
        let rows = sqlx::query!("SELECT tag_id FROM song_tag_refs WHERE song_id = $1", song_id)
            .fetch_all(&self.pool).await?;
        let result = rows.into_iter().map(|x| x.tag_id).collect();
        Ok(result)
    }

    async fn list_origin_info_by_song_id(&self, song_id: i64) -> sqlx::Result<Vec<SongOriginInfo>> {
        sqlx::query_as!(SongOriginInfo, "SELECT * FROM song_origin_info WHERE song_id = $1", song_id)
            .fetch_all(&self.pool).await
    }

    async fn list_production_crew_by_song_id(&self, song_id: i64) -> sqlx::Result<Vec<SongProductionCrew>> {
        sqlx::query_as!(SongProductionCrew, "SELECT * FROM song_production_crew WHERE song_id = $1", song_id)
            .fetch_all(&self.pool).await
    }

    async fn list_by_ids(&self, ids: &[i64]) -> sqlx::Result<Vec<Self::Entity>> {
        sqlx::query_as!(
            Song, "SELECT * FROM songs WHERE id = ANY($1)",
            ids
        ).fetch_all(&self.pool).await
    }
}
