//! Songs as report targets.

use super::{ContentAction, OwnerMessage, UgcAdapter, UgcTarget};
use crate::db::song::SongDao;
use crate::db::CrudDao;
use crate::service::notification::{to_plain_text, ContentIntent};
use crate::service::song;
use crate::web::state::AppState;
use sqlx::{PgPool, PgTransaction};
use tracing::warn;

/// Hide the song from everyone but its uploader, until a modification passes review.
const HIDE: ContentAction = ContentAction { id: "hide", penalty: true };
/// Show a hidden song again.
const RESTORE: ContentAction = ContentAction { id: "restore", penalty: false };
const ACTIONS: &[ContentAction] = &[HIDE, RESTORE];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SongAdapter;

impl UgcAdapter for SongAdapter {
    fn name(&self) -> &'static str {
        "song"
    }

    fn actions(&self) -> &'static [ContentAction] {
        ACTIONS
    }

    async fn load(&self, pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
        Ok(SongDao::get_by_id(pool, id).await?.map(|x| UgcTarget {
            owner_uid: x.uploader_uid,
            title: x.title,
            cover_url: Some(x.cover_art_url),
            display_id: x.display_id,
            is_public: !x.is_hidden,
            is_hidden: x.is_hidden,
        }))
    }

    /// A visible song can be hidden; a hidden one restored.
    fn content_actions(&self, target: &UgcTarget) -> Vec<ContentAction> {
        if target.is_hidden { vec![RESTORE] } else { vec![HIDE] }
    }

    async fn apply(&self, tx: &mut PgTransaction<'_>, id: i64, actions: &[ContentAction]) -> anyhow::Result<()> {
        song::set_hidden(tx, id, actions.contains(&HIDE)).await?;
        Ok(())
    }

    async fn refresh(&self, state: &AppState, id: i64) {
        match SongDao::get_by_id(&state.sql_pool, id).await {
            Ok(Some(x)) => song::refresh_after_visibility_change(state, &x).await,
            Ok(None) => {}
            Err(e) => warn!(song_id = id, "Failed to load the song to refresh: {e:?}"),
        }
    }

    fn mention(&self, title: Option<&str>) -> String {
        match title {
            Some(x) => format!("作品《{}》", to_plain_text(x)),
            None => "作品".to_string(),
        }
    }

    fn owner_message(&self, id: i64, target: &UgcTarget, actions: &[ContentAction], reason_suffix: &str) -> Option<OwnerMessage> {
        let title = to_plain_text(&target.title);
        let mut data = serde_json::Map::new();
        data.insert("song_id".into(), id.into());
        let intent = ContentIntent::new("creation.artwork.view", data);
        if actions.contains(&HIDE) {
            Some(OwnerMessage {
                notification_type: "governance.content_hidden",
                title: "作品已被隐藏",
                body: format!("你的作品《{title}》已被隐藏，目前只有你自己能看到。修改后重新提交审核，通过后即恢复显示。{reason_suffix}"),
                intent,
            })
        } else if actions.contains(&RESTORE) {
            Some(OwnerMessage {
                notification_type: "governance.content_restored",
                title: "作品已恢复显示",
                body: format!("你的作品《{title}》已恢复显示。{reason_suffix}"),
                intent,
            })
        } else {
            None
        }
    }
}
