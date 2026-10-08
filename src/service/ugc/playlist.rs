//! Playlists as report targets.

use super::{Changed, ContentAction, OwnerMessage, UgcTarget};
use crate::db::playlist::{Playlist, PlaylistDao};
use crate::db::CrudDao;
use crate::service::notification::{to_plain_text, ContentIntent};
use crate::service::playlist;
use crate::web::state::AppState;
use sqlx::{PgPool, PgTransaction};

pub async fn load(pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
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
pub fn content_actions(target: &UgcTarget) -> Vec<ContentAction> {
    if target.is_hidden {
        vec![ContentAction::Restore]
    } else {
        vec![ContentAction::Hide]
    }
}

pub async fn apply(tx: &mut PgTransaction<'_>, id: i64, actions: &[ContentAction]) -> anyhow::Result<Option<Changed>> {
    let hidden = actions.contains(&ContentAction::Hide);
    Ok(playlist::set_hidden(tx, id, hidden).await?.map(Changed::Playlist))
}

pub async fn refresh(state: &AppState, playlist: &Playlist) {
    playlist::refresh_after_visibility_change(state, playlist).await
}

pub fn mention(title: Option<&str>) -> String {
    match title {
        Some(x) => format!("歌单《{}》", to_plain_text(x)),
        None => "歌单".to_string(),
    }
}

pub fn owner_message(id: i64, target: &UgcTarget, actions: &[ContentAction], reason_suffix: &str) -> Option<OwnerMessage> {
    let title = to_plain_text(&target.title);
    let mut data = serde_json::Map::new();
    data.insert("playlist_id".into(), id.into());
    let intent = ContentIntent::new("playlist.view", data);
    if actions.contains(&ContentAction::Hide) {
        Some(OwnerMessage {
            notification_type: "governance.content_hidden",
            title: "歌单已被隐藏",
            body: format!("你的歌单《{title}》已被隐藏，目前只有你自己能看到。{reason_suffix}"),
            intent,
        })
    } else if actions.contains(&ContentAction::Restore) {
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
