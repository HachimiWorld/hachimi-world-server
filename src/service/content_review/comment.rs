//! Comments waiting for a content check.

use super::ReviewAdapter;
use crate::db::comment::CommentId;
use crate::db::user::UserId;
use crate::service::comment::{self, CommentTarget};
use sqlx::PgPool;

/// Comments on one kind of target, checked as that kind's subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentReview(CommentTarget);

impl CommentReview {
    pub const SONG: Self = Self(CommentTarget::Song);
}

impl ReviewAdapter for CommentReview {
    fn subject(&self) -> &'static str {
        self.0.subject()
    }

    async fn load_pending(&self, pool: &PgPool, id: i64) -> sqlx::Result<Option<(UserId, String)>> {
        comment::load_pending(pool, CommentId(id)).await
    }

    async fn settle(&self, pool: &PgPool, id: i64, pass: bool) -> sqlx::Result<()> {
        comment::settle_pending(pool, CommentId(id), pass).await
    }
}
