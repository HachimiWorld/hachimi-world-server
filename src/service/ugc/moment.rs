//! Moments as report targets.

use super::{ContentAction, OwnerMessage, Preview, UgcAdapter, UgcPreview, UgcTarget};
use crate::db::moment::{Moment, MomentDao, MomentId};
use crate::service::moment::{self, quote, StoredImage};
use crate::service::notification::ContentIntent;
use crate::web::state::AppState;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, PgTransaction};

/// Hide the moment from everyone but its author.
const HIDE: ContentAction = ContentAction { id: "hide", penalty: true };
/// Show a hidden moment again.
const RESTORE: ContentAction = ContentAction { id: "restore", penalty: false };
const ACTIONS: &[ContentAction] = &[HIDE, RESTORE];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MomentAdapter;

/// How a moment looks: its text and the thumbnails of its images.
/// @since 261106
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MomentPreview {
    pub id: MomentId,
    /// May be empty.
    pub content: String,
    pub image_thumb_urls: Vec<String>,
}

impl Preview for MomentPreview {
    fn headline(&self) -> &str {
        &self.content
    }
}

pub fn preview(x: &Moment) -> UgcPreview {
    let images: Vec<StoredImage> = serde_json::from_value(x.images.clone()).unwrap_or_default();
    UgcPreview::Moment(MomentPreview {
        id: x.id,
        content: x.content.clone(),
        image_thumb_urls: images.into_iter().map(|x| x.thumb_url).collect(),
    })
}

fn view_intent(id: i64) -> ContentIntent {
    let mut data = serde_json::Map::new();
    data.insert("moment_id".into(), id.into());
    ContentIntent::new("moment.view", data)
}

/// 动态「…」, or just 动态 when it has no text.
fn named(content: &str) -> String {
    if content.trim().is_empty() { "动态".to_string() } else { format!("动态「{}」", quote(content)) }
}

impl UgcAdapter for MomentAdapter {
    fn name(&self) -> &'static str {
        "moment"
    }

    fn actions(&self) -> &'static [ContentAction] {
        ACTIONS
    }

    /// Deleted moments are gone.
    async fn load(&self, pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
        Ok(MomentDao::get_by_id(pool, MomentId(id)).await?.filter(|x| x.delete_time.is_none()).map(|x| UgcTarget {
            owner_uid: x.user_id,
            is_public: x.is_visible(),
            is_hidden: x.is_hidden,
            preview: preview(&x),
        }))
    }

    /// A visible moment can be hidden; a hidden one restored.
    fn content_actions(&self, target: &UgcTarget) -> Vec<ContentAction> {
        if target.is_hidden { vec![RESTORE] } else { vec![HIDE] }
    }

    async fn apply(&self, tx: &mut PgTransaction<'_>, id: i64, actions: &[ContentAction]) -> anyhow::Result<()> {
        moment::set_hidden(tx, MomentId(id), actions.contains(&HIDE)).await
    }

    /// Moments aren't cached or indexed.
    async fn refresh(&self, _state: &AppState, _id: i64) {}

    fn mention(&self, target: Option<&UgcTarget>) -> String {
        match target {
            Some(x) => named(x.preview.headline()),
            None => "动态".to_string(),
        }
    }

    fn owner_message(&self, id: i64, target: &UgcTarget, actions: &[ContentAction], reason_suffix: &str) -> Option<OwnerMessage> {
        let name = named(target.preview.headline());
        let intent = Some(view_intent(id));
        if actions.contains(&HIDE) {
            Some(OwnerMessage {
                notification_type: "governance.content_hidden",
                title: "动态已被隐藏",
                body: format!("你的{name}已被隐藏，目前只有你自己能看到。{reason_suffix}"),
                intent,
            })
        } else if actions.contains(&RESTORE) {
            Some(OwnerMessage {
                notification_type: "governance.content_restored",
                title: "动态已恢复显示",
                body: format!("你的{name}已恢复显示。{reason_suffix}"),
                intent,
            })
        } else {
            None
        }
    }
}
