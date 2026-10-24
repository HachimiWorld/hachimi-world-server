//! Comments on songs.

use super::{CommentTargetAdapter, TargetInfo};
use crate::db::comment::CommentId;
use crate::db::song::{ISongDao, SongDao, SongId};
use crate::service::content_check;
use crate::service::notification::ContentIntent;
use crate::service::ugc::{self, UgcPreview};
use sqlx::PgPool;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SongComments;

impl CommentTargetAdapter for SongComments {
    fn name(&self) -> &'static str {
        "song"
    }

    fn subject(&self) -> &'static str {
        content_check::SUBJECT_SONG_COMMENT
    }

    async fn load_many(&self, pool: &PgPool, target_ids: &[i64]) -> sqlx::Result<HashMap<i64, TargetInfo>> {
        let ids: Vec<SongId> = target_ids.iter().map(|x| SongId(*x)).collect();
        Ok(SongDao::list_by_ids(pool, &ids).await?.into_iter().map(|x| (x.id.0, TargetInfo {
            owner_uid: x.uploader_uid,
            is_hidden: x.is_hidden,
            preview: ugc::song_preview(&x),
        })).collect())
    }

    /// Plays the song and opens its comments.
    fn view_intent(&self, on: &UgcPreview, comment_id: CommentId) -> Option<ContentIntent> {
        let UgcPreview::Song(song) = on else { return None };
        let mut data = serde_json::Map::new();
        data.insert("song_display_id".into(), song.display_id.clone().into());
        data.insert("comment_id".into(), comment_id.0.into());
        Some(ContentIntent::new("song.comment.view", data))
    }
}
