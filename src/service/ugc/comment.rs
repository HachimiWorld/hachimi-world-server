//! Comments as report targets.

use super::{ContentAction, OwnerMessage, UgcAdapter, UgcTarget};
use crate::db::comment::{CommentDao, CommentId, STATUS_PUBLISHED};
use crate::db::song::{SongDao, SongId};
use crate::db::CrudDao;
use crate::service::comment::{self, CommentTarget};
use crate::service::notification::{to_plain_text, ContentIntent};
use crate::web::state::AppState;
use sqlx::{PgPool, PgTransaction};

/// Hide the comment from everyone but its author.
const HIDE: ContentAction = ContentAction { id: "hide", penalty: true };
/// Show a hidden comment again.
const RESTORE: ContentAction = ContentAction { id: "restore", penalty: false };
const ACTIONS: &[ContentAction] = &[HIDE, RESTORE];
/// How much of a comment its mentions quote.
const QUOTE_CHARS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentAdapter;

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

    /// The comment's text is its title; the cover and display id are those of the song it's on.
    /// Only a published comment that isn't deleted can be reported.
    async fn load(&self, pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
        let Some(x) = CommentDao::get_by_id(pool, CommentId(id)).await? else {
            return Ok(None);
        };
        let song = match CommentTarget::parse(&x.target_type) {
            Some(CommentTarget::Song) => SongDao::get_by_id(pool, SongId(x.target_id)).await?,
            None => None,
        };
        Ok(Some(UgcTarget {
            owner_uid: x.user_id,
            is_public: x.status == STATUS_PUBLISHED && !x.is_hidden && x.delete_time.is_none(),
            is_hidden: x.is_hidden,
            title: x.content,
            cover_url: song.as_ref().map(|x| x.cover_art_url.clone()),
            display_id: song.map(|x| x.display_id).unwrap_or_default(),
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

    fn mention(&self, title: Option<&str>) -> String {
        match title {
            Some(x) => format!("评论「{}」", quote(x)),
            None => "评论".to_string(),
        }
    }

    fn owner_message(&self, id: i64, target: &UgcTarget, actions: &[ContentAction], reason_suffix: &str) -> Option<OwnerMessage> {
        let text = quote(&target.title);
        let mut data = serde_json::Map::new();
        data.insert("song_display_id".into(), target.display_id.clone().into());
        data.insert("comment_id".into(), id.into());
        let intent = ContentIntent::new("song.comment.view", data);
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
