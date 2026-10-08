//! Users as report targets: their profile is the content.

use super::{Changed, ContentAction, OwnerMessage, UgcTarget};
use crate::db::user::{User, UserDao};
use crate::db::CrudDao;
use crate::service::notification::{to_plain_text, ContentIntent};
use crate::service::user::{self, ProfileField};
use crate::web::state::AppState;
use sqlx::{PgPool, PgTransaction};

pub async fn load(pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
    Ok(UserDao::get_by_id(pool, id).await?.map(|x| UgcTarget {
        owner_uid: x.id,
        title: x.username,
        cover_url: x.avatar_url,
        display_id: String::new(),
        is_public: true,
        is_hidden: false,
    }))
}

/// Any part of the profile can be reset, several at once.
pub fn content_actions(_target: &UgcTarget) -> Vec<ContentAction> {
    vec![ContentAction::ResetAvatar, ContentAction::ResetBio, ContentAction::ResetUsername]
}

pub async fn apply(tx: &mut PgTransaction<'_>, id: i64, actions: &[ContentAction]) -> anyhow::Result<Option<Changed>> {
    let fields: Vec<ProfileField> = actions.iter().filter_map(|x| match x {
        ContentAction::ResetAvatar => Some(ProfileField::Avatar),
        ContentAction::ResetBio => Some(ProfileField::Bio),
        ContentAction::ResetUsername => Some(ProfileField::Username),
        _ => None,
    }).collect();
    Ok(user::reset_profile(tx, id, &fields).await?.map(Changed::User))
}

pub async fn refresh(state: &AppState, user: &User) {
    user::refresh_after_profile_change(state, user).await
}

pub fn mention(title: Option<&str>) -> String {
    match title {
        Some(x) => format!("用户「{}」", to_plain_text(x)),
        None => "用户".to_string(),
    }
}

pub fn owner_message(actions: &[ContentAction], reason_suffix: &str) -> Option<OwnerMessage> {
    let fields: Vec<&str> = actions.iter().filter_map(|x| match x {
        ContentAction::ResetAvatar => Some("头像"),
        ContentAction::ResetBio => Some("简介"),
        ContentAction::ResetUsername => Some("昵称"),
        _ => None,
    }).collect();
    if fields.is_empty() {
        return None;
    }
    Some(OwnerMessage {
        notification_type: "governance.profile_reset",
        title: "个人资料已被重置",
        body: format!("你的{}已被重置。{reason_suffix}", fields.join("、")),
        intent: ContentIntent::new("profile.edit", serde_json::Map::new()),
    })
}
