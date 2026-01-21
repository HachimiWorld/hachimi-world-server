use crate::db::CrudDao;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, PgTransaction, QueryBuilder};
use std::collections::HashMap;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Song {
    pub id: i64,
    pub display_id: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    /// Artist is deprecated because the artist might be a group of people
    #[deprecated]
    pub artist: String,
    pub file_url: String,
    pub cover_art_url: String,
    pub lyrics: String,
    pub duration_seconds: i32,
    pub uploader_uid: i64,
    pub creation_type: i32,
    /// Play count is deprecated because it should be got from play history
    #[deprecated(since = "20250925")]
    pub play_count: i64,
    // Like count is deprecated because it should be got from like history
    #[deprecated(since = "20250925")]
    pub like_count: i64,
    pub is_private: bool,
    pub release_time: DateTime<Utc>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
    // Since 251105
    pub explicit: Option<bool>,
    // Since 251105
    pub gain: Option<f32>,
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

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SongLike {
    pub song_id: i64,
    pub user_id: i64,
    pub create_time: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SongPlay {
    pub id: i64,
    pub song_id: i64,
    pub user_id: Option<i64>,
    pub anonymous_uid: Option<i64>,
    pub create_time: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SongExternalLink {
    pub id: i64,
    pub song_id: i64,
    pub platform: String,
    pub url: String,
}

pub struct SongDao;

pub trait ISongDao<'e, E>: CrudDao<'e, E>
where
    E: PgExecutor<'e>,
{
    fn get_by_display_id(executor: E, display_id: &str) -> impl Future<Output = sqlx::Result<Option<Song>>>;
    fn list_tags_by_song_id(executor: E, song_id: i64) -> impl Future<Output = sqlx::Result<Vec<i64>>>;
    fn list_origin_info_by_song_id(executor: E, song_id: i64) -> impl Future<Output = sqlx::Result<Vec<SongOriginInfo>>>;
    fn list_origin_info_by_song_ids(executor: E, song_ids: &[i64]) -> impl Future<Output = sqlx::Result<Vec<SongOriginInfo>>>;
    fn list_production_crew_by_song_id(executor: E, song_id: i64) -> impl Future<Output = sqlx::Result<Vec<SongProductionCrew>>>;
    fn list_production_crew_by_song_ids(executor: E, song_ids: &[i64]) -> impl Future<Output = sqlx::Result<Vec<SongProductionCrew>>>;
    fn list_external_link_by_song_id(executor: E, song_id: i64) -> impl Future<Output = sqlx::Result<Vec<SongExternalLink>>>;
    fn list_external_link_by_song_ids(executor: E, song_ids: &[i64]) -> impl Future<Output = sqlx::Result<Vec<SongExternalLink>>>;
    fn list_by_ids(executor: E, ids: &[i64]) -> impl Future<Output = sqlx::Result<Vec<Self::Entity>>>;
    fn list_by_create_time_after(executor: E, create_time: DateTime<Utc>, limit: i64) -> impl Future<Output = sqlx::Result<Vec<Self::Entity>>>;
    fn list_by_create_time_before(executor: E, create_time: DateTime<Utc>, limit: i64) -> impl Future<Output = sqlx::Result<Vec<Self::Entity>>>;
    fn page_by_user(executor: E, user_id: i64, page: i64, size: i64) -> impl Future<Output = sqlx::Result<Vec<Self::Entity>>>;
    fn count_by_user(executor: E, user_id: i64) -> impl Future<Output = sqlx::Result<i64>>;
    fn count_likes(executor: E, song_id: i64) -> impl Future<Output = sqlx::Result<i64>>;
    fn count_plays(executor: E, song_id: i64) -> impl Future<Output = sqlx::Result<i64>>;
    fn count_plays_batch(executor: E, song_ids: &[i64]) -> impl Future<Output = sqlx::Result<HashMap<i64, i64>>>;
    fn insert_likes(executor: E, values: &[SongLike]) -> impl Future<Output = sqlx::Result<()>>;
    fn is_liked(executor: E, song_id: i64, user_id: i64) -> impl Future<Output = sqlx::Result<bool>>;
    fn delete_like(executor: E, song_id: i64, user_id: i64) -> impl Future<Output = sqlx::Result<()>>;
    fn insert_plays(executor: E, values: &[SongPlay]) -> impl Future<Output = sqlx::Result<()>>;
    fn cursor_plays(executor: E, user_id: i64, max_create_time: DateTime<Utc>, size: usize) -> impl Future<Output = sqlx::Result<Vec<SongPlay>>>;
    fn delete_play(executor: E, id: i64, user_id: i64) -> impl Future<Output = sqlx::Result<()>>;
}

impl<'e, E> CrudDao<'e, E> for SongDao
where
    E: PgExecutor<'e>,
{
    type Entity = Song;

    async fn list(executor: E) -> sqlx::Result<Vec<Self::Entity>> {
        sqlx::query_as!(Song, "SELECT * FROM songs")
            .fetch_all(executor)
            .await
    }

    async fn page(executor: E, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        todo!()
    }

    async fn get_by_id(executor: E, id: i64) -> sqlx::Result<Option<Self::Entity>> {
        sqlx::query_as!(Song, "SELECT * FROM songs WHERE id = $1", id)
            .fetch_optional(executor)
            .await
    }

    async fn update_by_id(executor: E, value: &Self::Entity) -> sqlx::Result<()> {
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
                update_time = $17,
                explicit = $18,
                gain = $19
            WHERE id = $20",
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
            value.explicit,
            value.gain,
            value.id
        )
            .execute(executor)
            .await?;
        Ok(())
    }

    async fn insert(executor: E, value: &Self::Entity) -> sqlx::Result<i64> {
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
                update_time,
                explicit,
                gain
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19) RETURNING id",
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
            value.explicit,
            value.gain
        ).fetch_one(executor).await.map(|x| x.id)
    }

    async fn delete_by_id(executor: E, id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM songs WHERE id = $1", id)
            .execute(executor)
            .await?;
        Ok(())
    }
}

impl<'e, E> ISongDao<'e, E> for SongDao
where
    E: PgExecutor<'e>,
{
    async fn get_by_display_id(executor: E, display_id: &str) -> sqlx::Result<Option<Song>> {
        sqlx::query_as!(
            Song,
            "SELECT * FROM songs WHERE display_id = $1",
            display_id
        )
            .fetch_optional(executor)
            .await
    }

    async fn list_tags_by_song_id(executor: E, song_id: i64) -> sqlx::Result<Vec<i64>> {
        let rows = sqlx::query!("SELECT tag_id FROM song_tag_refs WHERE song_id = $1", song_id)
            .fetch_all(executor).await?;
        let result = rows.into_iter().map(|x| x.tag_id).collect();
        Ok(result)
    }
    
    async fn list_origin_info_by_song_id(executor: E, song_id: i64) -> sqlx::Result<Vec<SongOriginInfo>> {
        sqlx::query_as!(SongOriginInfo, "SELECT * FROM song_origin_info WHERE song_id = $1", song_id)
            .fetch_all(executor).await
    }

    async fn list_origin_info_by_song_ids(executor: E, song_ids: &[i64]) -> sqlx::Result<Vec<SongOriginInfo>> {
        if song_ids.is_empty() { return Ok(vec![]) }
        sqlx::query_as!(SongOriginInfo, "SELECT * FROM song_origin_info WHERE song_id = ANY($1)", song_ids)
            .fetch_all(executor).await
    }


    async fn list_production_crew_by_song_id(executor: E, song_id: i64) -> sqlx::Result<Vec<SongProductionCrew>> {
        sqlx::query_as!(SongProductionCrew, "SELECT * FROM song_production_crew WHERE song_id = $1", song_id)
            .fetch_all(executor).await
    }

    async fn list_production_crew_by_song_ids(executor: E, song_ids: &[i64]) -> sqlx::Result<Vec<SongProductionCrew>> {
        if song_ids.is_empty() { return Ok(vec![]) }
        sqlx::query_as!(SongProductionCrew, "SELECT * FROM song_production_crew WHERE song_id = ANY($1)", song_ids)
            .fetch_all(executor).await
    }

    async fn list_external_link_by_song_id(executor: E, song_id: i64) -> sqlx::Result<Vec<SongExternalLink>> {
        sqlx::query_as!(SongExternalLink, "SELECT * FROM song_external_links WHERE song_id = $1", song_id)
            .fetch_all(executor).await
    }

    async fn list_external_link_by_song_ids(executor: E, song_ids: &[i64]) -> sqlx::Result<Vec<SongExternalLink>> {
        if song_ids.is_empty() { return Ok(vec![]) }
        sqlx::query_as!(SongExternalLink, "SELECT * FROM song_external_links WHERE song_id = ANY($1)", song_ids)
            .fetch_all(executor).await
    }

    async fn list_by_ids(executor: E, ids: &[i64]) -> sqlx::Result<Vec<Self::Entity>> {
        if ids.is_empty() { return Ok(vec![]) }
        sqlx::query_as!(
            Song, "SELECT * FROM songs WHERE id = ANY($1)",
            ids
        ).fetch_all(executor).await
    }

    async fn list_by_create_time_after(executor: E, create_time: DateTime<Utc>, limit: i64) -> sqlx::Result<Vec<Self::Entity>> {
        sqlx::query_as!(Song, "SELECT * FROM songs WHERE create_time > $1 ORDER BY create_time ASC LIMIT $2", create_time, limit)
            .fetch_all(executor).await
    }

    async fn list_by_create_time_before(executor: E, create_time: DateTime<Utc>, limit: i64) -> sqlx::Result<Vec<Self::Entity>> {
        sqlx::query_as!(Song, "SELECT * FROM songs WHERE create_time < $1 ORDER BY create_time DESC LIMIT $2", create_time, limit)
            .fetch_all(executor).await
    }

    async fn page_by_user(executor: E, user_id: i64, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>> {
        sqlx::query_as!(
            Song,
            "SELECT * FROM songs WHERE uploader_uid = $1 ORDER BY id DESC LIMIT $2 OFFSET $3",
            user_id,
            size,
            page * size
        ).fetch_all(executor).await
    }

    async fn count_by_user(executor: E, user_id: i64) -> sqlx::Result<i64> {
        sqlx::query!(
            "SELECT COUNT(*) FROM songs WHERE uploader_uid = $1",
            user_id
        ).fetch_one(executor).await.map(|r| r.count.unwrap_or(0))
    }

    async fn count_likes(executor: E, song_id: i64) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(1) FROM song_likes WHERE song_id = $1", song_id)
            .fetch_one(executor)
            .await.map(|x| x.count).map(|x| x.unwrap_or(0))
    }

    async fn count_plays(executor: E, song_id: i64) -> sqlx::Result<i64> {
        sqlx::query!("SELECT COUNT(1) FROM song_plays WHERE song_id = $1", song_id)
            .fetch_one(executor)
            .await.map(|x| x.count).map(|x| x.unwrap_or(0))
    }

    async fn count_plays_batch(executor: E, song_ids: &[i64]) -> sqlx::Result<HashMap<i64, i64>> {
        if song_ids.is_empty() { return Ok(HashMap::new()) }
        let result = sqlx::query!("SELECT song_id, COUNT(*) FROM song_plays WHERE song_id = ANY($1) GROUP BY song_id", song_ids)
            .fetch_all(executor)
            .await?
            .into_iter()
            .map(|x| (x.song_id, x.count.unwrap_or(0)))
            .collect::<HashMap<_, _>>();
        Ok(result)
    }

    async fn insert_likes(executor: E, values: &[SongLike]) -> sqlx::Result<()> {
        let mut builder = QueryBuilder::new("INSERT INTO song_likes (song_id, user_id, create_time)");
        builder.push_values(values, |mut b, x| {
            b.push_bind(x.song_id);
            b.push_bind(x.user_id);
            b.push_bind(x.create_time);
        }).build().execute(executor).await?;
        Ok(())
    }

    async fn is_liked(executor: E, song_id: i64, user_id: i64) -> sqlx::Result<bool> {
        let count = sqlx::query!("SELECT COUNT(1) FROM song_likes WHERE song_id = $1 AND user_id = $2", song_id, user_id)
            .fetch_one(executor).await?
            .count.map(|x| x == 1).unwrap_or(false);
        Ok(count)
    }
    async fn delete_like(executor: E, song_id: i64, user_id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM song_likes WHERE song_id = $1 AND user_id = $2", song_id, user_id)
            .execute(executor)
            .await?;
        Ok(())
    }

    async fn insert_plays(executor: E, values: &[SongPlay]) -> sqlx::Result<()> {
        let ids = values.iter().map(|x| x.song_id).collect::<Vec<_>>();
        let user_ids = values.iter().map(|x| x.user_id).collect::<Vec<_>>();
        let anonymous_uids = values.iter().map(|x| x.anonymous_uid).collect::<Vec<_>>();
        let create_times = values.iter().map(|x| x.create_time).collect::<Vec<_>>();
        sqlx::query!(
            "INSERT INTO song_plays (song_id, user_id, anonymous_uid, create_time)
            SELECT * FROM UNNEST($1::bigint[], $2::bigint[], $3::bigint[], $4::timestamptz[])",
            &ids[..], &user_ids as &[Option<i64>], &anonymous_uids as &[Option<i64>], &create_times[..]
        ).execute(executor).await?;
        Ok(())
    }
    async fn cursor_plays(executor: E, user_id: i64, max_create_time: DateTime<Utc>, size: usize) -> sqlx::Result<Vec<SongPlay>> {
        sqlx::query_as!(
            SongPlay,
            "SELECT * FROM song_plays WHERE user_id = $1 AND create_time < $2 ORDER BY create_time DESC LIMIT $3",
            user_id,
            max_create_time,
            size as i64
        ).fetch_all(executor).await
    }

    async fn delete_play(executor: E, id: i64, user_id: i64) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM song_plays WHERE id = $1 AND user_id = $2", id, user_id)
            .execute(executor).await?;
        Ok(())
    }
}

impl<'e> SongDao {
    pub(crate) async fn update_song_production_crew(
        executor: &mut PgTransaction<'e>,
        song_id: i64,
        values: &[SongProductionCrew],
    ) -> sqlx::Result<()> {
        sqlx::query!(
            "DELETE FROM song_production_crew WHERE song_id = $1",
            song_id
        ).execute(&mut **executor).await?;
        for x in values {
            sqlx::query!(
                "INSERT INTO song_production_crew (
                    song_id,
                    role,
                    uid,
                    person_name
                ) VALUES ($1, $2, $3, $4)",
                song_id,
                x.role,
                x.uid,
                x.person_name
            ).execute(&mut **executor).await?;
        }
        Ok(())
    }

    pub(crate) async fn update_song_origin_info(
        executor: &mut PgTransaction<'e>,
        song_id: i64,
        values: &[SongOriginInfo],
    ) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM song_origin_info WHERE song_id = $1", song_id)
            .execute(&mut **executor)
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
                song_id,
                x.origin_type,
                x.origin_song_id,
                x.origin_title,
                x.origin_artist,
                x.origin_url
            )
                .execute(&mut **executor)
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn update_song_tags(executor: &mut PgTransaction<'e>, song_id: i64, tags: Vec<i64>) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM song_tag_refs WHERE song_id = $1", song_id)
            .execute(&mut **executor)
            .await?;
        for tag_id in tags {
            sqlx::query!(
                "INSERT INTO song_tag_refs (song_id, tag_id) VALUES ($1, $2)",
                song_id, tag_id
            ).execute(&mut **executor).await?;
        }
        Ok(())
    }

    pub async fn update_song_external_links(executor: &mut PgTransaction<'e>, song_id: i64, values: &[SongExternalLink]) -> sqlx::Result<()> {
        sqlx::query!("DELETE FROM song_external_links WHERE song_id = $1", song_id).execute(&mut **executor).await?;
        sqlx::query!("INSERT INTO song_external_links (song_id, platform, url) SELECT * FROM UNNEST($1::bigint[], $2::text[], $3::text[])",
            &values.iter().map(|_| song_id).collect::<Vec<_>>(),
            &values.iter().map(|x| x.platform.clone()).collect::<Vec<_>>(),
            &values.iter().map(|x| x.url.clone()).collect::<Vec<_>>()
        ).execute(&mut **executor).await?;
        Ok(())
    }
}