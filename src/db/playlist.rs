use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use crate::db::CrudDao;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub user_id: i64,
    pub cover_url: Option<String>,
    pub is_public: bool,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PlaylistSong {
    pub playlist_id: i64,
    pub song_id: i64,
    pub order_index: i32,
    pub add_time: DateTime<Utc>,
}

pub struct PlaylistDao {
    pool: PgPool,
}

impl PlaylistDao {
    pub fn new(pool: PgPool) -> Self {
        PlaylistDao { pool }
    }
}

pub trait IPlaylistDao: CrudDao {
    async fn remove_song(&self, playlist_id: i64, song_id: i64) -> sqlx::Result<()>;
    async fn add_song(&self, value: &PlaylistSong) -> sqlx::Result<()>;
    async fn list_songs(&self, playlist_id: i64) -> sqlx::Result<Vec<PlaylistSong>>;
    async fn count_songs(&self, playlist_id: i64) -> sqlx::Result<i64>;
    async fn list_by_user(&self, user_id: i64) -> sqlx::Result<Vec<Playlist>>;
    async fn count_by_user(&self, user_id: i64) -> sqlx::Result<i64>;
}

impl CrudDao for PlaylistDao {
    type Entity = Playlist;

    async fn list(&self) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn page(&self, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(&self, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        sqlx::query_as!(Self::Entity, "SELECT * FROM playlists WHERE id = $1", id)
            .fetch_optional(&self.pool)
            .await
    }

    async fn update_by_id(&self, value: &Self::Entity) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE playlists SET
                name = $1,
                description = $2,
                user_id = $3,
                cover_url = $4,
                is_public = $5,
                create_time = $6,
                update_time = $7
            WHERE id = $8",
            value.name,
            value.description,
            value.user_id,
            value.cover_url,
            value.is_public,
            value.create_time,
            value.update_time,
            value.id,
        ).execute(&self.pool).await?;
        Ok(())
    }

    async fn insert(&self, value: &Self::Entity) -> sqlx::Result<i64> {
        sqlx::query!(
            "INSERT INTO playlists (
               name,
               description,
               user_id,
               cover_url,
               is_public,
               create_time,
               update_time
            ) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
            value.name,
            value.description,
            value.user_id,
            value.cover_url,
            value.is_public,
            value.create_time,
            value.update_time
        ).fetch_one(&self.pool).await
            .map(|x| x.id)
    }

    async fn delete_by_id(&self, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM playlist_songs WHERE playlist_id = $1", id)
            .execute(&self.pool).await?;
        sqlx::query!("DELETE FROM playlists WHERE id = $1", id)
            .execute(&self.pool).await?;
        Ok(())
    }
}

impl IPlaylistDao for PlaylistDao {
    async fn remove_song(&self, playlist_id: i64, song_id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM playlist_songs WHERE playlist_id = $1 AND song_id = $2", playlist_id, song_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn add_song(&self, value: &PlaylistSong) -> sqlx::Result<()> {
        sqlx::query!(
            "INSERT INTO playlist_songs (playlist_id, song_id, order_index, add_time) VALUES ($1, $2, $3, $4)",
            value.playlist_id,
            value.song_id,
            value.order_index,
            value.add_time,
        ).execute(&self.pool)
            .await?;
        Ok(())
    }
    async fn list_songs(&self, playlist_id: i64) -> sqlx::Result<Vec<PlaylistSong>> {
        sqlx::query_as!(PlaylistSong, "SELECT * FROM playlist_songs WHERE playlist_id = $1", playlist_id)
            .fetch_all(&self.pool)
            .await
    }

    async fn count_songs(&self, playlist_id: i64) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(1) FROM playlist_songs WHERE playlist_id = $1", playlist_id)
            .fetch_one(&self.pool)
            .await
            .map(|x| x.count.unwrap_or(0))
    }

    async fn list_by_user(&self, user_id: i64) -> sqlx::Result<Vec<Playlist>> {
        sqlx::query_as!(Playlist, "SELECT * FROM playlists WHERE user_id = $1", user_id)
            .fetch_all(&self.pool)
            .await
    }

    async fn count_by_user(&self, user_id: i64) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(1) FROM playlists WHERE user_id = $1", user_id)
            .fetch_one(&self.pool)
            .await
            .map(|x| x.count.unwrap_or(0))
    }
}