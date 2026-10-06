//! Likes other users gave to a user's songs, read from `song_likes` and `songs`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

/// The likes one song received.
/// @since 261006
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedLikeGroup {
    pub song_id: i64,
    pub song_display_id: String,
    pub song_title: String,
    pub cover_art_url: String,
    pub like_count: i64,
    pub latest_like_time: DateTime<Utc>,
}

/// @since 261006
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedLike {
    pub song_id: i64,
    pub user_id: i64,
    pub create_time: DateTime<Utc>,
}

pub struct ReceivedLikeDao;

impl ReceivedLikeDao {
    /// Likes on songs uploaded by `uploader_uid`, excluding their own, created after `after`.
    pub async fn count_after<'e>(executor: impl PgExecutor<'e>, uploader_uid: i64, after: DateTime<Utc>) -> sqlx::Result<i64> {
        sqlx::query!(
            r#"SELECT COUNT(*) AS "count!" FROM song_likes l
            JOIN songs s ON s.id = l.song_id
            WHERE s.uploader_uid = $1 AND l.user_id <> $1 AND l.create_time > $2"#,
            uploader_uid, after
        ).fetch_one(executor).await.map(|x| x.count)
    }

    /// Songs of `uploader_uid` with likes from others, most recently liked first. Pass the
    /// latest like time and song id of the previous page's last group as `before`.
    pub async fn list_groups<'e>(
        executor: impl PgExecutor<'e>,
        uploader_uid: i64,
        before: Option<(DateTime<Utc>, i64)>,
        limit: i64,
    ) -> sqlx::Result<Vec<ReceivedLikeGroup>> {
        let (before_time, before_song_id) = before.unzip();
        sqlx::query_as!(
            ReceivedLikeGroup,
            r#"SELECT
                s.id AS song_id,
                s.display_id AS song_display_id,
                s.title AS song_title,
                s.cover_art_url,
                COUNT(*) AS "like_count!",
                MAX(l.create_time) AS "latest_like_time!"
            FROM songs s
            JOIN song_likes l ON l.song_id = s.id
            WHERE s.uploader_uid = $1 AND l.user_id <> $1
            GROUP BY s.id, s.display_id, s.title, s.cover_art_url
            HAVING $2::timestamptz IS NULL OR (MAX(l.create_time), s.id) < ($2, $3::bigint)
            ORDER BY MAX(l.create_time) DESC, s.id DESC
            LIMIT $4"#,
            uploader_uid, before_time, before_song_id, limit
        ).fetch_all(executor).await
    }

    /// The latest `per_song` likes of each song, excluding the uploader's own.
    pub async fn list_latest_by_songs<'e>(
        executor: impl PgExecutor<'e>,
        song_ids: &[i64],
        per_song: i64,
    ) -> sqlx::Result<Vec<ReceivedLike>> {
        sqlx::query_as!(
            ReceivedLike,
            r#"SELECT song_id AS "song_id!", user_id AS "user_id!", create_time AS "create_time!" FROM (
                SELECT l.song_id, l.user_id, l.create_time,
                    ROW_NUMBER() OVER (PARTITION BY l.song_id ORDER BY l.create_time DESC, l.user_id DESC) AS rn
                FROM song_likes l
                JOIN songs s ON s.id = l.song_id
                WHERE l.song_id = ANY($1) AND l.user_id <> s.uploader_uid
            ) t
            WHERE rn <= $2
            ORDER BY song_id, create_time DESC, user_id DESC"#,
            song_ids, per_song
        ).fetch_all(executor).await
    }
}
