//! The kinds of user-generated content that can be reported, and what a decision can do to each.
//!
//! [UgcKind] is the only place that matches on the kind: each method forwards to the kind's own
//! adapter (`song.rs`, `playlist.rs`, `user.rs`), which knows that content and nothing else. The
//! report flow only talks to [UgcKind]. Adding a kind means a new variant and a new adapter; the
//! compiler lists every match left to fill in.

mod playlist;
mod song;
mod user;

use crate::db::playlist::Playlist;
use crate::db::song::Song;
use crate::db::user::User;
use crate::service::notification::ContentIntent;
use crate::web::state::AppState;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, PgTransaction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UgcKind {
    Song,
    Playlist,
    User,
}

/// A target in the shape the report flow needs, whatever its kind: shown in the report queue,
/// used to check who may report it, and kept as a snapshot with each action.
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

/// What to tell the owner about a decision on their content.
pub struct OwnerMessage {
    pub notification_type: &'static str,
    pub title: &'static str,
    pub body: String,
    pub intent: ContentIntent,
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

    /// Reads the target from its own table and describes it as a [UgcTarget]. None if it doesn't
    /// exist.
    pub async fn load(self, pool: &PgPool, id: i64) -> sqlx::Result<Option<UgcTarget>> {
        match self {
            Self::Song => song::load(pool, id).await,
            Self::Playlist => playlist::load(pool, id).await,
            Self::User => user::load(pool, id).await,
        }
    }

    /// Content actions a decision can take on `target` now.
    pub fn content_actions(self, target: &UgcTarget) -> Vec<ContentAction> {
        match self {
            Self::Song => song::content_actions(target),
            Self::Playlist => playlist::content_actions(target),
            Self::User => user::content_actions(target),
        }
    }

    /// Takes `actions`, already checked against [Self::content_actions], in the decision's
    /// transaction.
    pub async fn apply(self, tx: &mut PgTransaction<'_>, id: i64, actions: &[ContentAction]) -> anyhow::Result<Option<Changed>> {
        if actions.is_empty() {
            return Ok(None);
        }
        match self {
            Self::Song => song::apply(tx, id, actions).await,
            Self::Playlist => playlist::apply(tx, id, actions).await,
            Self::User => user::apply(tx, id, actions).await,
        }
    }
}

impl UgcKind {
    /// How the target is named in user-facing text, such as 作品《X》, or just 作品 if it's gone.
    pub fn mention(self, target: Option<&UgcTarget>) -> String {
        let title = target.map(|x| x.title.as_str());
        match self {
            Self::Song => song::mention(title),
            Self::Playlist => playlist::mention(title),
            Self::User => user::mention(title),
        }
    }

    /// Tells the owner what `actions` did to their content. None if they did nothing.
    /// `reason_suffix` is appended to the body as is.
    pub fn owner_message(self, id: i64, target: &UgcTarget, actions: &[ContentAction], reason_suffix: &str) -> Option<OwnerMessage> {
        match self {
            Self::Song => song::owner_message(id, target, actions, reason_suffix),
            Self::Playlist => playlist::owner_message(id, target, actions, reason_suffix),
            Self::User => user::owner_message(actions, reason_suffix),
        }
    }
}

impl Changed {
    /// Refreshes caches and the search index after the decision commits.
    pub async fn refresh(&self, state: &AppState) {
        match self {
            Self::Song(x) => song::refresh(state, x).await,
            Self::Playlist(x) => playlist::refresh(state, x).await,
            Self::User(x) => user::refresh(state, x).await,
        }
    }
}
