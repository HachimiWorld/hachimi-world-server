use crate::db::CrudDao;
use crate::db::song::SongId;
use chrono::{DateTime, Utc};
use crate::db::user::UserId;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, PgTransaction};
use std::collections::HashMap;

define_id!(
    /// `playlists.id`
    PlaylistId
);

macro_rules! query_playlists {
    () => {
        sqlx::query_as!(
            Playlist, r#"
            SELECT
                id as "id: _",
                name,
                description,
                user_id as "user_id: _",
                cover_url,
                is_public,
                is_hidden,
                create_time,
                update_time
            FROM playlists
            "#
        )
    };
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            Playlist, r#"
            SELECT
                id as "id: _",
                name,
                description,
                user_id as "user_id: _",
                cover_url,
                is_public,
                is_hidden,
                create_time,
                update_time
            FROM playlists
            "# + $extra,
            $($arg),*
        )
    };
}

macro_rules! query_playlist_songs {
    () => {
        sqlx::query_as!(
            PlaylistSong, r#"
            SELECT
                playlist_id as "playlist_id: _",
                song_id as "song_id: _",
                order_index,
                add_time
            FROM playlist_songs
            "#
        )
    };
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            PlaylistSong, r#"
            SELECT
                playlist_id as "playlist_id: _",
                song_id as "song_id: _",
                order_index,
                add_time
            FROM playlist_songs
            "# + $extra,
            $($arg),*
        )
    };
}

macro_rules! query_favorite_playlists {
    () => {
        sqlx::query_as!(
            FavoritePlaylist, r#"
            SELECT
                user_id as "user_id: _",
                playlist_id as "playlist_id: _",
                order_index,
                add_time
            FROM favorite_playlists
            "#
        )
    };
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            FavoritePlaylist, r#"
            SELECT
                user_id as "user_id: _",
                playlist_id as "playlist_id: _",
                order_index,
                add_time
            FROM favorite_playlists
            "# + $extra,
            $($arg),*
        )
    };
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    pub description: Option<String>,
    pub user_id: UserId,
    pub cover_url: Option<String>,
    pub is_public: bool,
    /// Hidden by the platform: only the owner can see it, and only [PlaylistDao::set_hidden]
    /// changes it.
    /// @since 261008
    #[serde(default)]
    pub is_hidden: bool,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PlaylistSong {
    pub playlist_id: PlaylistId,
    pub song_id: SongId,
    pub order_index: i32,
    pub add_time: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct FavoritePlaylist {
    pub user_id: UserId,
    pub playlist_id: PlaylistId,
    pub order_index: i32,
    pub add_time: DateTime<Utc>,
}

pub struct PlaylistDao;

pub trait IPlaylistDao<'e, E>: CrudDao<'e, E>
where
    E: PgExecutor<'e>,
{
    fn remove_song(executor: E, playlist_id: PlaylistId, song_id: SongId) -> impl Future<Output=sqlx::Result<()>> + Send;
    fn add_song(executor: E, value: &PlaylistSong) -> impl Future<Output=sqlx::Result<()>> + Send;
    fn list_songs(executor: E, playlist_id: PlaylistId) -> impl Future<Output=sqlx::Result<Vec<PlaylistSong>>> + Send;
    fn count_songs(executor: E, playlist_ids: &[PlaylistId]) -> impl Future<Output=sqlx::Result<HashMap<PlaylistId, i64>>> + Send;
    fn list_by_user(executor: E, user_id: UserId) -> impl Future<Output=sqlx::Result<Vec<Playlist>>> + Send;
    fn list_by_ids(executor: E, ids: &[PlaylistId]) -> impl Future<Output=sqlx::Result<Vec<Playlist>>> + Send;
    fn list_containing(executor: E, song_id: SongId, user_id: UserId) -> impl Future<Output=sqlx::Result<Vec<Playlist>>> + Send;
    fn count_by_user(executor: E, user_id: UserId) -> impl Future<Output=sqlx::Result<i64>> + Send;
    fn page_favorites(executor: E, user_id: UserId, page_index: i64, page_size: i64) -> impl Future<Output=sqlx::Result<Vec<FavoritePlaylist>>> + Send;
    fn count_favorites(executor: E, user_id: UserId) -> impl Future<Output=sqlx::Result<i64>> + Send;
    fn add_favorite(executor: E, value: &FavoritePlaylist) -> impl Future<Output=sqlx::Result<()>> + Send;
    fn get_favorite(executor: E, user_id: UserId, playlist_id: PlaylistId) -> impl Future<Output=sqlx::Result<Option<FavoritePlaylist>>> + Send;
    fn remove_favorite(executor: E, user_id: UserId, playlist_id: PlaylistId) -> impl Future<Output=sqlx::Result<()>> + Send;
}

impl<'e, E> CrudDao<'e, E> for PlaylistDao
where
    E: PgExecutor<'e>,
{
    type Entity = Playlist;
    type Id = PlaylistId;

    async fn list(_executor: E) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn page(_executor: E, _page: i64, _size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(executor: E, id: PlaylistId) -> sqlx::Result<Option<Self::Entity>> {
        query_playlists!("WHERE id = $1", id as PlaylistId)
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
            value.user_id as UserId,
            value.cover_url,
            value.is_public,
            value.create_time,
            value.update_time,
            value.id as PlaylistId,
        ).execute(executor).await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> sqlx::Result<PlaylistId> {
        sqlx::query!(
            "INSERT INTO playlists (
               name,
               description,
               user_id,
               cover_url,
               is_public,
               is_hidden,
               create_time,
               update_time
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id as \"id: PlaylistId\"",
            value.name,
            value.description,
            value.user_id as UserId,
            value.cover_url,
            value.is_public,
            value.is_hidden,
            value.create_time,
            value.update_time
        ).fetch_one(executor).await
            .map(|x| x.id)
    }

    async fn delete_by_id(_executor: E, _id: PlaylistId) -> sqlx::Result<()> {
        todo!()
    }
}

impl<'e, E> IPlaylistDao<'e, E> for PlaylistDao
where
    E: PgExecutor<'e>,
{
    async fn remove_song(executor: E, playlist_id: PlaylistId, song_id: SongId) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM playlist_songs WHERE playlist_id = $1 AND song_id = $2", playlist_id as PlaylistId, song_id as SongId)
            .execute(executor)
            .await?;
        Ok(())
    }

    async fn add_song(executor: E, value: &PlaylistSong) -> sqlx::Result<()> {
        sqlx::query!(
            "INSERT INTO playlist_songs (playlist_id, song_id, order_index, add_time) VALUES ($1, $2, $3, $4)",
            value.playlist_id as PlaylistId,
            value.song_id as SongId,
            value.order_index,
            value.add_time,
        ).execute(executor)
            .await?;
        Ok(())
    }
    async fn list_songs(executor: E, playlist_id: PlaylistId) -> sqlx::Result<Vec<PlaylistSong>> {
        query_playlist_songs!("WHERE playlist_id = $1 ORDER BY order_index", playlist_id as PlaylistId)
            .fetch_all(executor)
            .await
    }

    async fn count_songs(executor: E, playlist_ids: &[PlaylistId]) -> sqlx::Result<HashMap<PlaylistId, i64>> {
        if playlist_ids.is_empty() { return Ok(HashMap::new()); }

        let result = sqlx::query!("SELECT playlist_id as \"playlist_id: PlaylistId\", COUNT(*) FROM playlist_songs WHERE playlist_id = ANY($1) GROUP BY playlist_id", playlist_ids as &[PlaylistId])
            .fetch_all(executor)
            .await?
            .into_iter()
            .map(|x| (x.playlist_id, x.count.unwrap_or(0)))
            .collect::<HashMap<_, _>>();
        Ok(result)
    }

    async fn list_by_user(executor: E, user_id: UserId) -> sqlx::Result<Vec<Playlist>> {
        query_playlists!("WHERE user_id = $1", user_id as UserId)
            .fetch_all(executor)
            .await
    }

    async fn list_by_ids(executor: E, ids: &[PlaylistId]) -> sqlx::Result<Vec<Playlist>> {
        if ids.is_empty() { return Ok(vec![]); }
        query_playlists!("WHERE id = ANY($1)", ids as &[PlaylistId])
            .fetch_all(executor)
            .await
    }

    async fn list_containing(executor: E, song_id: SongId, user_id: UserId) -> sqlx::Result<Vec<Playlist>> {
        query_playlists!(
            "p JOIN playlist_songs ps ON ps.playlist_id = p.id WHERE ps.song_id = $1 AND p.user_id = $2",
            song_id as SongId,
            user_id as UserId
        )
            .fetch_all(executor)
            .await
    }

    async fn count_by_user(executor: E, user_id: UserId) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(1) FROM playlists WHERE user_id = $1", user_id as UserId)
            .fetch_one(executor)
            .await
            .map(|x| x.count.unwrap_or(0))
    }

    async fn page_favorites(executor: E, user_id: UserId, page_index: i64, page_size: i64) -> sqlx::Result<Vec<FavoritePlaylist>> {
        let offset = page_index * page_size;
        query_favorite_playlists!(
            "WHERE user_id = $1 ORDER BY add_time DESC LIMIT $2 OFFSET $3",
            user_id as UserId, page_size, offset
        ).fetch_all(executor).await
    }

    async fn count_favorites(executor: E, user_id: UserId) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(*) FROM favorite_playlists WHERE user_id = $1", user_id as UserId)
            .fetch_one(executor)
            .await
            .map(|x| x.count.unwrap_or(0))
    }

    async fn add_favorite(executor: E, value: &FavoritePlaylist) -> sqlx::Result<()> {
        // insert and ignore duplicate favorites
        sqlx::query!(
            "INSERT INTO favorite_playlists (user_id, playlist_id, order_index, add_time) VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, playlist_id) DO NOTHING",
            value.user_id as UserId,
            value.playlist_id as PlaylistId,
            value.order_index,
            value.add_time
        ).execute(executor).await?;
        Ok(())
    }

    async fn remove_favorite(executor: E, user_id: UserId, playlist_id: PlaylistId) -> sqlx::Result<()> {
        sqlx::query!(
            "DELETE FROM favorite_playlists WHERE user_id = $1 AND playlist_id = $2",
            user_id as UserId,
            playlist_id as PlaylistId
        ).execute(executor).await?;
        Ok(())
    }

    async fn get_favorite(executor: E, user_id: UserId, playlist_id: PlaylistId) -> sqlx::Result<Option<FavoritePlaylist>> {
        query_favorite_playlists!(
            "WHERE user_id = $1 AND playlist_id = $2",
            user_id as UserId,
            playlist_id as PlaylistId
        )
        .fetch_optional(executor)
        .await
    }
}

impl<'e> PlaylistDao {
    pub async fn update_songs_orders(tx: &mut PgTransaction<'e>, values: &[PlaylistSong]) -> sqlx::Result<()> {
        for value in values {
            sqlx::query!(
                "UPDATE playlist_songs SET order_index = $1 WHERE playlist_id = $2 AND song_id = $3",
                value.order_index,
                value.playlist_id as PlaylistId,
                value.song_id as SongId,
            ).execute(&mut **tx).await?;
        }
        Ok(())
    }

    pub async fn delete_cascade_by_id(tx: &mut PgTransaction<'e>, id: PlaylistId) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM playlist_songs WHERE playlist_id = $1", id as PlaylistId)
            .execute(&mut **tx).await?;
        sqlx::query!("DELETE FROM playlists WHERE id = $1", id as PlaylistId)
            .execute(&mut **tx).await?;
        Ok(())
    }
}

impl PlaylistDao {
    /// Returns whether the playlist exists.
    /// @since 261008
    pub async fn set_hidden<'e>(executor: impl PgExecutor<'e>, id: PlaylistId, hidden: bool) -> sqlx::Result<bool> {
        sqlx::query!("UPDATE playlists SET is_hidden = $2 WHERE id = $1", id as PlaylistId, hidden)
            .execute(executor).await.map(|x| x.rows_affected() > 0)
    }
}
