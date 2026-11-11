//! A song attached to a moment. Stores `{ "id": <song id> }`; shown as the song's public detail,
//! the same as in song lists, so the client can play it.

use super::AttachmentAdapter;
use crate::db::song::{SongDao, SongId};
use crate::db::user::UserId;
use crate::db::CrudDao;
use crate::service::content_review::ReviewItem;
use crate::service::moment::{AttachmentView, KnownAttachment};
use crate::service::song;
use crate::web::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SongAttachment;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SongData {
    id: SongId,
}

impl AttachmentAdapter for SongAttachment {
    fn name(&self) -> &'static str {
        "song"
    }

    /// Any song that isn't hidden.
    async fn prepare(&self, state: &AppState, _uid: UserId, data: &Value) -> anyhow::Result<Option<Value>> {
        let Ok(data) = serde_json::from_value::<SongData>(data.clone()) else {
            return Ok(None);
        };
        match SongDao::get_by_id(&state.sql_pool, data.id).await? {
            Some(x) if !x.is_hidden => Ok(Some(serde_json::to_value(SongData { id: x.id })?)),
            _ => Ok(None),
        }
    }

    /// Songs are reviewed before they're published.
    fn review_items(&self, _data: &Value) -> anyhow::Result<Vec<ReviewItem>> {
        Ok(vec![])
    }

    async fn show_many(&self, state: &AppState, data: &[&Value]) -> anyhow::Result<Vec<AttachmentView>> {
        let ids: Vec<Option<SongId>> = data.iter()
            .map(|x| serde_json::from_value::<SongData>((*x).clone()).ok().map(|x| x.id))
            .collect();
        let mut wanted: Vec<SongId> = ids.iter().flatten().copied().collect();
        wanted.sort();
        wanted.dedup();
        let songs = song::get_public_detail_with_cache(state.redis_conn.clone(), &state.sql_pool, &wanted).await?;
        Ok(ids.into_iter()
            .map(|id| AttachmentView::Known(KnownAttachment::Song(id.and_then(|id| songs.get(&id)).cloned())))
            .collect())
    }
}
