use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, PgTransaction};
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

pub struct PlaylistDao;

pub trait IPlaylistDao<'e, E>: CrudDao<'e, E>
where
    E: PgExecutor<'e>,
{
    async fn remove_song(executor: E, playlist_id: i64, song_id: i64) -> sqlx::Result<()>;
    async fn add_song(executor: E, value: &PlaylistSong) -> sqlx::Result<()>;
    async fn list_songs(executor: E, playlist_id: i64) -> sqlx::Result<Vec<PlaylistSong>>;
    async fn count_songs(executor: E, playlist_id: i64) -> sqlx::Result<i64>;
    async fn list_by_user(executor: E, user_id: i64) -> sqlx::Result<Vec<Playlist>>;
    async fn count_by_user(executor: E, user_id: i64) -> sqlx::Result<i64>;
}

impl<'e, E> CrudDao<'e, E> for PlaylistDao
where
    E: PgExecutor<'e>,
{
    type Entity = Playlist;

    async fn list(executor: E) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn page(executor: E, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(executor: E, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        sqlx::query_as!(Self::Entity, "SELECT * FROM playlists WHERE id = $1", id)
            .fetch_optional(executor)
            .await
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> sqlx::Result<()> {
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
        ).execute(executor).await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> sqlx::Result<i64> {
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
        ).fetch_one(executor).await
            .map(|x| x.id)
    }

    async fn delete_by_id(executor: E, id: i64) -> sqlx::Result<()> {
        todo!()
    }
}

impl <'e, E> IPlaylistDao<'e, E> for PlaylistDao
where E: PgExecutor<'e>,{
    async fn remove_song(executor: E, playlist_id: i64, song_id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM playlist_songs WHERE playlist_id = $1 AND song_id = $2", playlist_id, song_id)
            .execute(executor)
            .await?;
        Ok(())
    }

    async fn add_song(executor: E, value: &PlaylistSong) -> sqlx::Result<()> {
        sqlx::query!(
            "INSERT INTO playlist_songs (playlist_id, song_id, order_index, add_time) VALUES ($1, $2, $3, $4)",
            value.playlist_id,
            value.song_id,
            value.order_index,
            value.add_time,
        ).execute(executor)
            .await?;
        Ok(())
    }
    async fn list_songs(executor: E, playlist_id: i64) -> sqlx::Result<Vec<PlaylistSong>> {
        sqlx::query_as!(PlaylistSong, "SELECT * FROM playlist_songs WHERE playlist_id = $1 ORDER BY order_index", playlist_id)
            .fetch_all(executor)
            .await
    }

    async fn count_songs(executor: E, playlist_id: i64) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(1) FROM playlist_songs WHERE playlist_id = $1", playlist_id)
            .fetch_one(executor)
            .await
            .map(|x| x.count.unwrap_or(0))
    }

    async fn list_by_user(executor: E, user_id: i64) -> sqlx::Result<Vec<Playlist>> {
        sqlx::query_as!(Playlist, "SELECT * FROM playlists WHERE user_id = $1", user_id)
            .fetch_all(executor)
            .await
    }

    async fn count_by_user(executor: E, user_id: i64) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(1) FROM playlists WHERE user_id = $1", user_id)
            .fetch_one(executor)
            .await
            .map(|x| x.count.unwrap_or(0))
    }
}

impl <'e> PlaylistDao {
    pub async fn update_songs_orders(tx: &mut PgTransaction<'e>, values: &[PlaylistSong]) -> sqlx::Result<()> {
        for value in values {
            sqlx::query!(
                "UPDATE playlist_songs SET order_index = $1 WHERE playlist_id = $2 AND song_id = $3",
                value.order_index,
                value.playlist_id,
                value.song_id,
            ).execute(&mut **tx).await?;
        }
        Ok(())
    }

    pub async fn delete_cascade_by_id(tx: &mut PgTransaction<'e>, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM playlist_songs WHERE playlist_id = $1", id)
            .execute(&mut **tx).await?;
        sqlx::query!("DELETE FROM playlists WHERE id = $1", id)
            .execute(&mut **tx).await?;
        Ok(())
    }
}
