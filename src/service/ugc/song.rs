//! Songs as report targets.

use super::{Changed, ContentAction, OwnerMessage, UgcTarget};
use crate::db::song::{Song, SongDao};
use crate::db::CrudDao;
use crate::service::notification::{to_plain_text, ContentIntent};
use crate::service::song;
use crate::web::state::AppState;
use sqlx::{PgPool, PgTransaction};

pub async fn load(pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
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
pub fn content_actions(target: &UgcTarget) -> Vec<ContentAction> {
    if target.is_hidden {
        vec![ContentAction::Restore]
    } else {
        vec![ContentAction::Hide]
    }
}

pub async fn apply(tx: &mut PgTransaction<'_>, id: i64, actions: &[ContentAction]) -> anyhow::Result<Option<Changed>> {
    let hidden = actions.contains(&ContentAction::Hide);
    Ok(song::set_hidden(tx, id, hidden).await?.map(Changed::Song))
}

pub async fn refresh(state: &AppState, song: &Song) {
    song::refresh_after_visibility_change(state, song).await
}

pub fn mention(title: Option<&str>) -> String {
    match title {
        Some(x) => format!("作品《{}》", to_plain_text(x)),
        None => "作品".to_string(),
    }
}

pub fn owner_message(id: i64, target: &UgcTarget, actions: &[ContentAction], reason_suffix: &str) -> Option<OwnerMessage> {
    let title = to_plain_text(&target.title);
    let mut data = serde_json::Map::new();
    data.insert("song_id".into(), id.into());
    let intent = ContentIntent::new("creation.artwork.view", data);
    if actions.contains(&ContentAction::Hide) {
        Some(OwnerMessage {
            notification_type: "governance.content_hidden",
            title: "作品已被隐藏",
            body: format!("你的作品《{title}》已被隐藏，目前只有你自己能看到。修改后重新提交审核，通过后即恢复显示。{reason_suffix}"),
            intent,
        })
    } else if actions.contains(&ContentAction::Restore) {
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
