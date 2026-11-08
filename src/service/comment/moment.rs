//! Comments on moments. Deleted moments are gone for comments too.

use super::{CommentTargetAdapter, TargetInfo};
use crate::db::comment::CommentId;
use crate::db::moment::{MomentDao, MomentId};
use crate::service::content_check;
use crate::service::moment::COMMENT_TARGET_TYPE;
use crate::service::notification::ContentIntent;
use crate::service::ugc::{self, UgcPreview};
use sqlx::PgPool;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MomentComments;

impl CommentTargetAdapter for MomentComments {
    fn name(&self) -> &'static str {
        COMMENT_TARGET_TYPE
    }

    fn subject(&self) -> &'static str {
        content_check::SUBJECT_MOMENT_COMMENT
    }

    async fn load_many(&self, pool: &PgPool, target_ids: &[i64]) -> sqlx::Result<HashMap<i64, TargetInfo>> {
        let ids: Vec<MomentId> = target_ids.iter().map(|x| MomentId(*x)).collect();
        Ok(MomentDao::list_by_ids(pool, &ids).await?.into_iter()
            .filter(|x| x.delete_time.is_none())
            .map(|x| (x.id.0, TargetInfo {
                owner_uid: x.user_id,
                is_hidden: x.is_hidden,
                preview: ugc::moment_preview(&x),
            }))
            .collect())
    }

    /// Opens the moment at its comments.
    fn view_intent(&self, on: &UgcPreview, comment_id: CommentId) -> Option<ContentIntent> {
        let UgcPreview::Moment(moment) = on else { return None };
        let mut data = serde_json::Map::new();
        data.insert("moment_id".into(), moment.id.0.into());
        data.insert("comment_id".into(), comment_id.0.into());
        Some(ContentIntent::new("moment.comment.view", data))
    }
}
