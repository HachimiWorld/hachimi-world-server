//! Comments waiting for a content check.

use super::ReviewAdapter;
use crate::service::comment::{self, CommentTarget, PendingComment};
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

    fn text(&self, payload: &Value) -> serde_json::Result<String> {
        PendingComment::deserialize(payload).map(|x| x.content)
    }

    async fn publish(&self, tx: &mut Transaction<'_, Postgres>, payload: &Value) -> anyhow::Result<()> {
        comment::publish_pending(tx, &PendingComment::deserialize(payload)?).await?;
        Ok(())
    }
}
