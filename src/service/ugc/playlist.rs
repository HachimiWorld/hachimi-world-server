//! Playlists as report targets.

use super::{ContentAction, OwnerMessage, UgcAdapter, UgcTarget};
use crate::db::playlist::PlaylistDao;
use crate::db::CrudDao;
use crate::service::notification::{to_plain_text, ContentIntent};
use crate::service::playlist;
use crate::web::state::AppState;
use sqlx::{PgPool, PgTransaction};
use tracing::warn;

/// Hide the playlist from everyone but its owner, until a contributor restores it.
const HIDE: ContentAction = ContentAction { id: "hide", penalty: true };
/// Show a hidden playlist again.
const RESTORE: ContentAction = ContentAction { id: "restore", penalty: false };
const ACTIONS: &[ContentAction] = &[HIDE, RESTORE];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaylistAdapter;

impl UgcAdapter for PlaylistAdapter {
    fn name(&self) -> &'static str {
        "playlist"
    }

    fn actions(&self) -> &'static [ContentAction] {
        ACTIONS
    }

    async fn load(&self, pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
        Ok(PlaylistDao::get_by_id(pool, id).await?.map(|x| UgcTarget {
            owner_uid: x.user_id,
            title: x.name,
            cover_url: x.cover_url,
            display_id: String::new(),
            is_public: x.is_public && !x.is_hidden,
            is_hidden: x.is_hidden,
        }))
    }

    /// A visible playlist can be hidden; a hidden one restored.
    fn content_actions(&self, target: &UgcTarget) -> Vec<ContentAction> {
        if target.is_hidden { vec![RESTORE] } else { vec![HIDE] }
    }

    async fn apply(&self, tx: &mut PgTransaction<'_>, id: i64, actions: &[ContentAction]) -> anyhow::Result<()> {
        playlist::set_hidden(tx, id, actions.contains(&HIDE)).await?;
        Ok(())
    }

    async fn refresh(&self, state: &AppState, id: i64) {
        match PlaylistDao::get_by_id(&state.sql_pool, id).await {
            Ok(Some(x)) => playlist::refresh_after_visibility_change(state, &x).await,
            Ok(None) => {}
            Err(e) => warn!(playlist_id = id, "Failed to load the playlist to refresh: {e:?}"),
        }
    }

    fn mention(&self, title: Option<&str>) -> String {
        match title {
            Some(x) => format!("歌单《{}》", to_plain_text(x)),
            None => "歌单".to_string(),
        }
    }

    fn owner_message(&self, id: i64, target: &UgcTarget, actions: &[ContentAction], reason_suffix: &str) -> Option<OwnerMessage> {
        let title = to_plain_text(&target.title);
        let mut data = serde_json::Map::new();
        data.insert("playlist_id".into(), id.into());
        let intent = ContentIntent::new("playlist.view", data);
        if actions.contains(&HIDE) {
            Some(OwnerMessage {
                notification_type: "governance.content_hidden",
                title: "歌单已被隐藏",
                body: format!("你的歌单《{title}》已被隐藏，目前只有你自己能看到。{reason_suffix}"),
                intent,
            })
        } else if actions.contains(&RESTORE) {
            Some(OwnerMessage {
                notification_type: "governance.content_restored",
                title: "歌单已恢复显示",
                body: format!("你的歌单《{title}》已恢复显示。{reason_suffix}"),
                intent,
            })
        } else {
            None
        }
    }
}
