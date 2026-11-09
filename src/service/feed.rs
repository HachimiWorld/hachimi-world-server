//! Streams that mix kinds of content, such as the square. Each entry is a [FeedItem] naming its
//! kind, so a stream can take in a new kind, such as new songs, without clients changing: they skip
//! kinds they don't know. A new kind is a new [FeedEntry] variant.
//!
//! Pages continue from an opaque cursor, so how a stream orders its kinds can change too.

use crate::db::moment::MomentId;
use crate::db::user::UserId;
use crate::service::moment::{self, MomentCursor, MomentItem};
use crate::web::state::AppState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One entry of a stream: `{ "time", "kind", "data" }`, where `kind` says what `data` is.
/// @since 261106
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FeedItem {
    /// When it happened, which orders the stream.
    pub time: DateTime<Utc>,
    #[serde(flatten)]
    pub entry: FeedEntry,
}

/// What an entry is, as `kind`, and the entry itself, as `data`. Clients skip kinds they don't
/// know.
/// @since 261106
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum FeedEntry {
    Moment(MomentItem),
}

pub struct FeedPage {
    pub items: Vec<FeedItem>,
    /// Pass it back for the next page; `None` when there's no more.
    pub next_cursor: Option<String>,
}

/// Where a page ended: the time, kind and id of its last entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub time: DateTime<Utc>,
    pub kind: String,
    pub id: i64,
}

impl Cursor {
    pub fn encode(&self) -> String {
        format!("{}.{}.{}", self.time.timestamp_micros(), self.kind, self.id)
    }

    pub fn decode(s: &str) -> Option<Self> {
        let mut parts = s.splitn(3, '.');
        let time = DateTime::from_timestamp_micros(parts.next()?.parse().ok()?)?;
        let kind = parts.next()?.to_string();
        let id = parts.next()?.parse().ok()?;
        Some(Cursor { time, kind, id })
    }
}

/// The square: everyone's moments, newest first. Moments of banned users are left out.
pub async fn square(state: &AppState, viewer_uid: Option<UserId>, cursor: Option<Cursor>, limit: i64) -> anyhow::Result<FeedPage> {
    let cursor = cursor.map(|x| MomentCursor { time: x.time, id: MomentId(x.id) });
    let page = moment::list_public(state, viewer_uid, cursor, limit).await?;
    let next_cursor = page.next_cursor.map(|x| Cursor { time: x.time, kind: "moment".to_string(), id: x.id.0 }.encode());
    let items = page.moments.into_iter().map(|x| FeedItem { time: x.create_time, entry: FeedEntry::Moment(x) }).collect();
    Ok(FeedPage { items, next_cursor })
}

#[cfg(test)]
mod tests {
    use super::Cursor;
    use chrono::{SubsecRound, Utc};

    #[test]
    fn cursors_read_back() {
        let cursor = Cursor { time: Utc::now().trunc_subsecs(6), kind: "moment".to_string(), id: 42 };
        assert_eq!(Cursor::decode(&cursor.encode()), Some(cursor));
        assert_eq!(Cursor::decode("x"), None);
        assert_eq!(Cursor::decode("1.moment"), None);
    }
}
