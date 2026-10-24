//! Comments as report targets.

use super::{ContentAction, LegacyCard, OwnerMessage, Preview, UgcAdapter, UgcPreview, UgcTarget};
use crate::db::comment::{CommentDao, CommentId, STATUS_PUBLISHED};
use crate::service::comment::{self, CommentTarget};
use crate::service::notification::to_plain_text;
use crate::web::state::AppState;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, PgTransaction};

/// Hide the comment from everyone but its author.
const HIDE: ContentAction = ContentAction { id: "hide", penalty: true };
/// Show a hidden comment again.
const RESTORE: ContentAction = ContentAction { id: "restore", penalty: false };
const ACTIONS: &[ContentAction] = &[HIDE, RESTORE];
/// How much of a comment its mentions quote.
const QUOTE_CHARS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommentAdapter;

/// How a comment looks: its text, and what it's on as that target's kind shows it.
/// @since 261021
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentPreview {
    pub content: String,
    /// What the comment is on; None if it's gone.
    pub on: Option<Box<UgcPreview>>,
}

impl Preview for CommentPreview {
    fn headline(&self) -> &str {
        &self.content
    }

    /// The comment's text over the card of what it's on.
    fn legacy_card(&self) -> LegacyCard {
        let on = self.on.as_ref().map(|x| x.legacy_card()).unwrap_or_default();
        LegacyCard { title: self.content.clone(), ..on }
    }
}

fn quote(content: &str) -> String {
    let text = to_plain_text(content).replace('\n', " ");
    if text.chars().count() > QUOTE_CHARS {
        format!("{}…", text.chars().take(QUOTE_CHARS).collect::<String>())
    } else {
        text
    }
}

impl UgcAdapter for CommentAdapter {
    fn name(&self) -> &'static str {
        "comment"
    }

    fn actions(&self) -> &'static [ContentAction] {
        ACTIONS
    }

    /// The preview shows what the comment is on, as that target's kind shows it. Only a
    /// published comment that isn't deleted can be reported.
    async fn load(&self, pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
        let Some(x) = CommentDao::get_by_id(pool, CommentId(id)).await? else {
            return Ok(None);
        };
        let on = match CommentTarget::parse(&x.target_type) {
            Some(target) => target.load(pool, x.target_id).await?.map(|x| Box::new(x.preview)),
            None => None,
        };
        Ok(Some(UgcTarget {
            owner_uid: x.user_id,
            is_public: x.status == STATUS_PUBLISHED && !x.is_hidden && x.delete_time.is_none(),
            is_hidden: x.is_hidden,
            preview: UgcPreview::Comment(CommentPreview { content: x.content, on }),
        }))
    }

    /// A visible comment can be hidden; a hidden one restored.
    fn content_actions(&self, target: &UgcTarget) -> Vec<ContentAction> {
        if target.is_hidden { vec![RESTORE] } else { vec![HIDE] }
    }

    async fn apply(&self, tx: &mut PgTransaction<'_>, id: i64, actions: &[ContentAction]) -> anyhow::Result<()> {
        comment::set_hidden(tx, CommentId(id), actions.contains(&HIDE)).await?;
        Ok(())
    }

    /// Comments aren't cached or indexed.
    async fn refresh(&self, _state: &AppState, _id: i64) {}

    fn mention(&self, target: Option<&UgcTarget>) -> String {
        match target.map(|x| x.preview.headline()) {
            Some(x) => format!("评论「{}」", quote(x)),
            None => "评论".to_string(),
        }
    }

    fn owner_message(&self, id: i64, target: &UgcTarget, actions: &[ContentAction], reason_suffix: &str) -> Option<OwnerMessage> {
        let text = quote(target.preview.headline());
        let intent = match &target.preview {
            UgcPreview::Comment(CommentPreview { on: Some(on), .. }) => CommentTarget::view_intent_of(on, CommentId(id)),
            _ => None,
        };
        if actions.contains(&HIDE) {
            Some(OwnerMessage {
                notification_type: "governance.content_hidden",
                title: "评论已被隐藏",
                body: format!("你的评论「{text}」已被隐藏，目前只有你自己能看到。{reason_suffix}"),
                intent,
            })
        } else if actions.contains(&RESTORE) {
            Some(OwnerMessage {
                notification_type: "governance.content_restored",
                title: "评论已恢复显示",
                body: format!("你的评论「{text}」已恢复显示。{reason_suffix}"),
                intent,
            })
        } else {
            None
        }
    }
}
