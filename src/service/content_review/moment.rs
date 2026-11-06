//! Moments waiting for content checks.

use super::{ReviewAdapter, ReviewItem};
use crate::service::content_check;
use crate::service::moment::{self, PendingMoment};
use serde::Deserialize;
use serde_json::Value;
use sqlx::{Postgres, Transaction};

/// Moments, which wait under [content_check::SUBJECT_MOMENT_TEXT] whatever they need checked. The
/// payload is a [PendingMoment].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MomentReview;

impl ReviewAdapter for MomentReview {
    fn subjects(&self) -> Vec<&'static str> {
        vec![content_check::SUBJECT_MOMENT_TEXT]
    }

    fn items(&self, payload: &Value) -> serde_json::Result<Vec<ReviewItem>> {
        PendingMoment::deserialize(payload)?.review_items().map_err(serde::de::Error::custom)
    }

    async fn publish(&self, tx: &mut Transaction<'_, Postgres>, payload: &Value) -> anyhow::Result<()> {
        moment::publish_pending(tx, &PendingMoment::deserialize(payload)?).await
    }

    /// A moment is longer than a comment, so its author is told rather than left wondering.
    async fn reject(&self, tx: &mut Transaction<'_, Postgres>, payload: &Value) -> anyhow::Result<()> {
        moment::reject_pending(tx, &PendingMoment::deserialize(payload)?).await
    }
}
