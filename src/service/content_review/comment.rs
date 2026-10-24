//! Comments waiting for a content check.

use super::ReviewAdapter;
use crate::db::comment::CommentId;
use crate::db::user::UserId;
use crate::service::comment::{self, CommentTarget, CommentTargetAdapter};
use sqlx::PgPool;
use strum::IntoEnumIterator;

/// Comments on every kind of target, each checked as its target kind's subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommentReview;

impl ReviewAdapter for CommentReview {
    fn subjects(&self) -> Vec<&'static str> {
        CommentTarget::iter().map(|x| x.subject()).collect()
    }

    async fn load_pending(&self, pool: &PgPool, id: i64) -> sqlx::Result<Option<(UserId, String)>> {
        comment::load_pending(pool, CommentId(id)).await
    }

    async fn settle(&self, pool: &PgPool, id: i64, pass: bool) -> sqlx::Result<()> {
        comment::settle_pending(pool, CommentId(id), pass).await
    }
}
