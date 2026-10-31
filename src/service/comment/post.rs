//! Comments on posts. Posts are written by contributors and can't be hidden.

use super::{CommentTargetAdapter, TargetInfo};
use crate::db::comment::CommentId;
use crate::db::post::PostDao;
use crate::service::content_check;
use crate::service::notification::ContentIntent;
use crate::service::ugc::{self, UgcPreview};
use sqlx::PgPool;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PostComments;

impl CommentTargetAdapter for PostComments {
    fn name(&self) -> &'static str {
        "post"
    }

    fn subject(&self) -> &'static str {
        content_check::SUBJECT_POST_COMMENT
    }

    async fn load_many(&self, pool: &PgPool, target_ids: &[i64]) -> sqlx::Result<HashMap<i64, TargetInfo>> {
        Ok(PostDao::list_by_ids(pool, target_ids).await?.into_iter().map(|x| (x.id, TargetInfo {
            owner_uid: x.author_uid,
            is_hidden: false,
            preview: ugc::post_preview(&x),
        })).collect())
    }

    /// Opens the post at its comments.
    fn view_intent(&self, on: &UgcPreview, comment_id: CommentId) -> Option<ContentIntent> {
        let UgcPreview::Post(post) = on else { return None };
        let mut data = serde_json::Map::new();
        data.insert("post_id".into(), post.id.into());
        data.insert("comment_id".into(), comment_id.0.into());
        Some(ContentIntent::new("post.comment.view", data))
    }
}
