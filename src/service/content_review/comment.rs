//! Comments waiting for a content check.

use super::{ReviewAdapter, ReviewItem};
use crate::service::comment::{self, CommentTarget, CommentTargetAdapter, PendingComment};
use serde::Deserialize;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use strum::IntoEnumIterator;

/// Comments on every kind of target, each checked as its target kind's subject. The payload is a
/// [PendingComment].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommentReview;

impl ReviewAdapter for CommentReview {
    fn subjects(&self) -> Vec<&'static str> {
        CommentTarget::iter().map(|x| x.subject()).collect()
    }

    fn items(&self, payload: &Value) -> serde_json::Result<Vec<ReviewItem>> {
        let comment = PendingComment::deserialize(payload)?;
        let subject = CommentTarget::parse(&comment.target_type)
            .map(|x| x.subject())
            .ok_or_else(|| serde::de::Error::custom(format!("unknown target type {}", comment.target_type)))?;
        Ok(vec![ReviewItem::Text { subject, text: comment.content }])
    }

    async fn publish(&self, tx: &mut Transaction<'_, Postgres>, payload: &Value) -> anyhow::Result<()> {
        comment::publish_pending(tx, &PendingComment::deserialize(payload)?).await?;
        Ok(())
    }

    /// Blocked comments are dropped without a word, as when blocked on posting.
    async fn reject(&self, _tx: &mut Transaction<'_, Postgres>, _payload: &Value) -> anyhow::Result<()> {
        Ok(())
    }
}
