//! Users as report targets: their profile is the content.

use super::{ContentAction, OwnerMessage, UgcAdapter, UgcTarget};
use crate::db::user::UserDao;
use crate::db::CrudDao;
use crate::service::notification::{to_plain_text, ContentIntent};
use crate::service::user::{self, ProfileField};
use crate::web::state::AppState;
use sqlx::{PgPool, PgTransaction};
use tracing::warn;

const RESET_AVATAR: ContentAction = ContentAction { id: "reset_avatar", penalty: true };
const RESET_BIO: ContentAction = ContentAction { id: "reset_bio", penalty: true };
/// Replace the username with a random one.
const RESET_USERNAME: ContentAction = ContentAction { id: "reset_username", penalty: true };
const ACTIONS: &[ContentAction] = &[RESET_AVATAR, RESET_BIO, RESET_USERNAME];

/// Each action, the profile field it resets, and how the owner's notification names it.
const FIELDS: &[(ContentAction, ProfileField, &str)] = &[
    (RESET_AVATAR, ProfileField::Avatar, "头像"),
    (RESET_BIO, ProfileField::Bio, "简介"),
    (RESET_USERNAME, ProfileField::Username, "昵称"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserAdapter;

impl UgcAdapter for UserAdapter {
    fn name(&self) -> &'static str {
        "user"
    }

    fn actions(&self) -> &'static [ContentAction] {
        ACTIONS
    }

    async fn load(&self, pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
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
    fn content_actions(&self, _target: &UgcTarget) -> Vec<ContentAction> {
        ACTIONS.to_vec()
    }

    async fn apply(&self, tx: &mut PgTransaction<'_>, id: i64, actions: &[ContentAction]) -> anyhow::Result<()> {
        let fields: Vec<ProfileField> = FIELDS.iter()
            .filter(|(action, _, _)| actions.contains(action))
            .map(|(_, field, _)| *field)
            .collect();
        user::reset_profile(tx, id, &fields).await?;
        Ok(())
    }

    async fn refresh(&self, state: &AppState, id: i64) {
        match UserDao::get_by_id(&state.sql_pool, id).await {
            Ok(Some(x)) => user::refresh_after_profile_change(state, &x).await,
            Ok(None) => {}
            Err(e) => warn!(uid = id, "Failed to load the user to refresh: {e:?}"),
        }
    }

    fn mention(&self, title: Option<&str>) -> String {
        match title {
            Some(x) => format!("用户「{}」", to_plain_text(x)),
            None => "用户".to_string(),
        }
    }

    fn owner_message(&self, _id: i64, _target: &UgcTarget, actions: &[ContentAction], reason_suffix: &str) -> Option<OwnerMessage> {
        let names: Vec<&str> = FIELDS.iter()
            .filter(|(action, _, _)| actions.contains(action))
            .map(|(_, _, name)| *name)
            .collect();
        if names.is_empty() {
            return None;
        }
        Some(OwnerMessage {
            notification_type: "governance.profile_reset",
            title: "个人资料已被重置",
            body: format!("你的{}已被重置。{reason_suffix}", names.join("、")),
            intent: ContentIntent::new("profile.edit", serde_json::Map::new()),
        })
    }
}
