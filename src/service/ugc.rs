//! The kinds of user-generated content that can be reported, and what a decision can do to each.
//! Adding a kind means a new variant here.

use crate::db::playlist::{Playlist, PlaylistDao};
use crate::db::song::{Song, SongDao};
use crate::db::user::{User, UserDao};
use crate::db::CrudDao;
use crate::service::user::ProfileField;
use crate::service::{playlist, song, user};
use crate::web::state::AppState;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, PgTransaction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UgcKind {
    Song,
    Playlist,
    User,
}

/// What a target looks like, shown in the report queue and kept as a snapshot with each action.
/// @since 261008
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UgcTarget {
    pub owner_uid: i64,
    /// Song title, playlist name or username.
    pub title: String,
    /// Song cover, playlist cover or avatar.
    pub cover_url: Option<String>,
    /// Song display id such as `JM-ABCD-123`; empty for other kinds.
    pub display_id: String,
    /// Whether people other than the owner can see it.
    pub is_public: bool,
    /// Hidden by the platform.
    /// @since 261008
    #[serde(default)]
    pub is_hidden: bool,
}

/// What a decision does to the reported content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentAction {
    /// Hide a song or playlist from everyone but its owner.
    Hide,
    /// Show a hidden song or playlist again.
    Restore,
    ResetAvatar,
    ResetBio,
    ResetUsername,
}

impl ContentAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "hide" => Some(Self::Hide),
            "restore" => Some(Self::Restore),
            "reset_avatar" => Some(Self::ResetAvatar),
            "reset_bio" => Some(Self::ResetBio),
            "reset_username" => Some(Self::ResetUsername),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hide => "hide",
            Self::Restore => "restore",
            Self::ResetAvatar => "reset_avatar",
            Self::ResetBio => "reset_bio",
            Self::ResetUsername => "reset_username",
        }
    }

    /// Acts against the content, so it needs an upheld report. Otherwise it undoes an earlier one.
    pub fn is_penalty(self) -> bool {
        self != Self::Restore
    }
}

/// The content after a decision changed it, to refresh caches and the search index once the
/// transaction commits.
pub enum Changed {
    Song(Song),
    Playlist(Playlist),
    User(User),
}

impl UgcKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "song" => Some(Self::Song),
            "playlist" => Some(Self::Playlist),
            "user" => Some(Self::User),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Song => "song",
            Self::Playlist => "playlist",
            Self::User => "user",
        }
    }

    pub async fn load(self, pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
        Ok(match self {
            Self::Song => SongDao::get_by_id(pool, id).await?.map(|x| UgcTarget {
                owner_uid: x.uploader_uid,
                title: x.title,
                cover_url: Some(x.cover_art_url),
                display_id: x.display_id,
                is_public: !x.is_hidden,
                is_hidden: x.is_hidden,
            }),
            Self::Playlist => PlaylistDao::get_by_id(pool, id).await?.map(|x| UgcTarget {
                owner_uid: x.user_id,
                title: x.name,
                cover_url: x.cover_url,
                display_id: String::new(),
                is_public: x.is_public && !x.is_hidden,
                is_hidden: x.is_hidden,
            }),
            Self::User => UserDao::get_by_id(pool, id).await?.map(|x| UgcTarget {
                owner_uid: x.id,
                title: x.username,
                cover_url: x.avatar_url,
                display_id: String::new(),
                is_public: true,
                is_hidden: false,
            }),
        })
    }

    /// Content actions a decision can take on `target` now.
    pub fn content_actions(self, target: &UgcTarget) -> Vec<ContentAction> {
        match self {
            Self::Song | Self::Playlist if target.is_hidden => vec![ContentAction::Restore],
            Self::Song | Self::Playlist => vec![ContentAction::Hide],
            Self::User => vec![ContentAction::ResetAvatar, ContentAction::ResetBio, ContentAction::ResetUsername],
        }
    }

    /// Takes `actions`, already checked against [Self::content_actions], in the decision's
    /// transaction.
    pub async fn apply(self, tx: &mut PgTransaction<'_>, id: i64, actions: &[ContentAction]) -> anyhow::Result<Option<Changed>> {
        if actions.is_empty() {
            return Ok(None);
        }
        Ok(match self {
            Self::Song => {
                let hidden = actions.contains(&ContentAction::Hide);
                song::set_hidden(tx, id, hidden).await?.map(Changed::Song)
            }
            Self::Playlist => {
                let hidden = actions.contains(&ContentAction::Hide);
                playlist::set_hidden(tx, id, hidden).await?.map(Changed::Playlist)
            }
            Self::User => {
                let fields: Vec<ProfileField> = actions.iter().filter_map(|x| match x {
                    ContentAction::ResetAvatar => Some(ProfileField::Avatar),
                    ContentAction::ResetBio => Some(ProfileField::Bio),
                    ContentAction::ResetUsername => Some(ProfileField::Username),
                    _ => None,
                }).collect();
                user::reset_profile(tx, id, &fields).await?.map(Changed::User)
            }
        })
    }
}

impl Changed {
    /// Refreshes caches and the search index after the decision commits.
    pub async fn refresh(&self, state: &AppState) {
        match self {
            Self::Song(x) => song::refresh_after_visibility_change(state, x).await,
            Self::Playlist(x) => playlist::refresh_after_visibility_change(state, x).await,
            Self::User(x) => user::refresh_after_profile_change(state, x).await,
        }
    }
}
