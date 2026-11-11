//! A playlist attached to a moment. Stores `{ "id": <playlist id> }`; shown as the playlist's
//! metadata, the same as in playlist lists.

use super::AttachmentAdapter;
use crate::db::playlist::{PlaylistDao, PlaylistId};
use crate::db::user::UserId;
use crate::db::CrudDao;
use crate::service::content_review::ReviewItem;
use crate::service::moment::{AttachmentView, KnownAttachment};
use crate::service::playlist;
use crate::web::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PlaylistAttachment;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlaylistData {
    id: PlaylistId,
}

impl AttachmentAdapter for PlaylistAttachment {
    fn name(&self) -> &'static str {
        "playlist"
    }

    /// Any public playlist that isn't hidden, as others couldn't see a private one.
    async fn prepare(&self, state: &AppState, _uid: UserId, data: &Value) -> anyhow::Result<Option<Value>> {
        let Ok(data) = serde_json::from_value::<PlaylistData>(data.clone()) else {
            return Ok(None);
        };
        match PlaylistDao::get_by_id(&state.sql_pool, data.id).await? {
            Some(x) if x.is_public && !x.is_hidden => Ok(Some(serde_json::to_value(PlaylistData { id: x.id })?)),
            _ => Ok(None),
        }
    }

    /// Playlists' names, descriptions and covers are checked where they're set.
    fn review_items(&self, _data: &Value) -> anyhow::Result<Vec<ReviewItem>> {
        Ok(vec![])
    }

    /// Ones made private or hidden since are shown as unavailable.
    async fn show_many(&self, state: &AppState, data: &[&Value]) -> anyhow::Result<Vec<AttachmentView>> {
        let ids: Vec<Option<PlaylistId>> = data.iter()
            .map(|x| serde_json::from_value::<PlaylistData>((*x).clone()).ok().map(|x| x.id))
            .collect();
        let mut wanted: Vec<PlaylistId> = ids.iter().flatten().copied().collect();
        wanted.sort();
        wanted.dedup();
        let playlists: HashMap<PlaylistId, _> = playlist::list_playlist_metadata(state.redis_conn.clone(), &state.sql_pool, &wanted, true).await?
            .into_iter()
            .map(|x| (x.id, x))
            .collect();
        Ok(ids.into_iter()
            .map(|id| AttachmentView::Known(KnownAttachment::Playlist(id.and_then(|id| playlists.get(&id)).cloned())))
            .collect())
    }
}
