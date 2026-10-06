use crate::service::message_center::{self, Channel, ReceivedLikeItem};
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{err, ok};
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// @since 261006
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/summary", get(summary))
        .route("/mark_read", post(mark_read))
        .route("/received_likes", get(received_likes))
}

/// @since 261006
#[derive(Debug, Serialize, Deserialize)]
pub struct SummaryResp {
    /// Sum of the unread counts below, for the entry badge.
    pub total_unread: i64,
    pub system_unread: i64,
    pub like_unread: i64,
    pub follow_unread: i64,
    /// Likes after this are unread.
    pub like_read_time: DateTime<Utc>,
    /// Followers who followed after this are new.
    pub follow_read_time: DateTime<Utc>,
}

async fn summary(claims: Claims, state: State<AppState>) -> WebResult<SummaryResp> {
    let x = message_center::summary(&state.sql_pool, claims.uid()).await?;
    ok!(SummaryResp {
        total_unread: x.system_unread + x.like_unread + x.follow_unread,
        system_unread: x.system_unread,
        like_unread: x.like_unread,
        follow_unread: x.follow_unread,
        like_read_time: x.like_read_time,
        follow_read_time: x.follow_read_time,
    })
}

/// @since 261006
#[derive(Debug, Serialize, Deserialize)]
pub struct MarkReadReq {
    /// `like` or `follow`. System notifications are marked through `/notification/*`.
    pub channel: String,
}

/// @since 261006
#[derive(Debug, Serialize, Deserialize)]
pub struct MarkReadResp {
    pub read_time: DateTime<Utc>,
}

async fn mark_read(claims: Claims, state: State<AppState>, req: Json<MarkReadReq>) -> WebResult<MarkReadResp> {
    let Some(channel) = Channel::parse(&req.channel) else {
        err!("invalid_channel", "Channel must be `like` or `follow`")
    };
    let read_time = message_center::mark_read(&state.sql_pool, claims.uid(), channel).await?;
    ok!(MarkReadResp { read_time })
}

/// @since 261006
#[derive(Debug, Serialize, Deserialize)]
pub struct ReceivedLikesReq {
    /// `latest_like_time` of the previous page's last item. Omit both for the first page.
    pub before_time: Option<DateTime<Utc>>,
    /// `song_id` of the previous page's last item.
    pub before_song_id: Option<i64>,
    /// 1..=50, defaults to 20.
    pub limit: Option<i64>,
}

/// @since 261006
#[derive(Debug, Serialize, Deserialize)]
pub struct ReceivedLikesResp {
    /// Most recently liked first.
    pub items: Vec<ReceivedLikeItem>,
    pub has_more: bool,
}

async fn received_likes(claims: Claims, state: State<AppState>, req: Query<ReceivedLikesReq>) -> WebResult<ReceivedLikesResp> {
    let before = match (req.before_time, req.before_song_id) {
        (Some(time), Some(song_id)) => Some((time, song_id)),
        (None, None) => None,
        _ => err!("invalid_cursor", "before_time and before_song_id must be given together"),
    };
    let limit = req.limit.unwrap_or(20).clamp(1, 50);
    let (items, has_more) = message_center::received_likes(&state.sql_pool, claims.uid(), before, limit).await?;
    ok!(ReceivedLikesResp { items, has_more })
}
