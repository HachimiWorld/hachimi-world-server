//! What a moment can carry besides text and images, such as a song or a playlist. Each type is
//! stored as `{ type, data }`, and only the type reads its `data`: a song or a playlist stores an
//! id, and a future type such as a poll can store whatever it needs.
//!
//! As with [crate::service::comment::CommentTarget], each [AttachmentKind] variant carries its
//! adapter and `enum_dispatch` forwards [AttachmentAdapter] to it. Adding a type means a new
//! adapter and a variant for it; [AttachmentKind::iter] finds it by itself.

mod playlist;
mod song;

use crate::db::user::UserId;
use crate::service::content_review::ReviewItem;
use crate::service::moment::AttachmentView;
use crate::web::state::AppState;
use enum_dispatch::enum_dispatch;
use serde_json::Value;
use strum::{EnumIter, IntoEnumIterator};

/// A type of attachment. Each variant carries the type's adapter, made by [Default] when
/// [EnumIter] lists the types.
/// @since 261106
#[enum_dispatch(AttachmentAdapter)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter)]
pub enum AttachmentKind {
    Song(song::SongAttachment),
    Playlist(playlist::PlaylistAttachment),
}

/// What every attachment type provides, implemented once per type in its own file.
// Only called through the concrete AttachmentKind, so the futures' auto traits (Send) are known
#[allow(async_fn_in_trait)]
#[enum_dispatch]
pub trait AttachmentAdapter {
    /// The type's name in the API and in storage, such as `song`.
    fn name(&self) -> &'static str;

    /// Reads `data` as `uid` sent it and returns what to store, or `None` if it isn't valid, such as
    /// pointing at something that doesn't exist or that others can't see.
    async fn prepare(&self, state: &AppState, uid: UserId, data: &Value) -> anyhow::Result<Option<Value>>;

    /// What in the stored `data` needs a content check before the moment is shown, such as a poll's
    /// options. Nothing for what was checked where it comes from, such as a song.
    fn review_items(&self, data: &Value) -> anyhow::Result<Vec<ReviewItem>>;

    /// Shows stored attachments of this type, all at once: for each, its own [AttachmentView]
    /// variant, with no data if what it points at can't be seen any more.
    async fn show_many(&self, state: &AppState, data: &[&Value]) -> anyhow::Result<Vec<AttachmentView>>;
}

impl AttachmentKind {
    pub fn parse(s: &str) -> Option<Self> {
        Self::iter().find(|x| x.name() == s)
    }
}

#[cfg(test)]
mod tests {
    use super::{AttachmentAdapter, AttachmentKind};
    use strum::IntoEnumIterator;

    #[test]
    fn every_kind_parses_back() {
        for kind in AttachmentKind::iter() {
            assert_eq!(AttachmentKind::parse(kind.name()), Some(kind));
        }
    }
}
