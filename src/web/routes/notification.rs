use crate::db::notification::Notification;
use crate::service::notification::{self, ContentIntent};
use crate::web::jwt::Claims;
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::state::AppState;
use crate::{common, err, ok};
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// @since 261005
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/list", get(list))
        .route("/detail", get(detail))
        .route("/unread_count", get(unread_count))
        .route("/mark_read", post(mark_read))
        .route("/mark_all_read", post(mark_all_read))
}

/// @since 261005
#[derive(Debug, Serialize, Deserialize)]
pub struct NotificationItem {
    /// UUIDv7. Newer notifications have greater ids.
    pub notification_id: Uuid,
    /// `<domain>.<event>`. Clients must still show notifications of unknown types.
    #[serde(rename = "type")]
    pub notification_type: String,
    pub title: String,
    pub body: String,
    /// Where "去查看" goes. Clients hide the button for actions they don't know.
    pub content_intent: Option<ContentIntent>,
    /// `null` if unread.
    pub read_time: Option<DateTime<Utc>>,
    pub create_time: DateTime<Utc>,
}

impl From<Notification> for NotificationItem {
    fn from(x: Notification) -> Self {
        Self {
            notification_id: x.id,
            notification_type: x.notification_type,
            title: x.title,
            body: x.body,
            content_intent: x.content_intent.as_ref().and_then(ContentIntent::from_stored),
            read_time: x.read_time,
            create_time: x.create_time,
        }
    }
}

fn parse_id(s: &str) -> Result<Uuid, WebError<CommonError>> {
    Uuid::parse_str(s).map_err(|_| common!("invalid_notification_id", "Invalid notification id"))
}

/// @since 261005
#[derive(Debug, Serialize, Deserialize)]
pub struct ListReq {
    /// The last `notification_id` of the previous page. Omit for the first page.
    pub before_id: Option<String>,
    /// 1..=50, defaults to 20.
    pub limit: Option<i64>,
}

/// @since 261005
#[derive(Debug, Serialize, Deserialize)]
pub struct ListResp {
    /// Newest first.
    pub items: Vec<NotificationItem>,
    pub has_more: bool,
}

async fn list(claims: Claims, state: State<AppState>, req: Query<ListReq>) -> WebResult<ListResp> {
    let before_id = req.before_id.as_deref().map(parse_id).transpose()?;
    let limit = req.limit.unwrap_or(20).clamp(1, 50);
    let (items, has_more) = notification::list(&state.sql_pool, claims.uid(), before_id, limit).await?;
    ok!(ListResp {
        items: items.into_iter().map(NotificationItem::from).collect(),
        has_more,
    })
}

/// @since 261005
#[derive(Debug, Serialize, Deserialize)]
pub struct DetailReq {
    pub notification_id: String,
}

async fn detail(claims: Claims, state: State<AppState>, req: Query<DetailReq>) -> WebResult<NotificationItem> {
    let id = parse_id(&req.notification_id)?;
    match notification::get(&state.sql_pool, claims.uid(), id).await? {
        Some(x) => ok!(x.into()),
        None => err!("notification_unavailable", "The notification does not exist or has expired"),
    }
}

/// @since 261005
#[derive(Debug, Serialize, Deserialize)]
pub struct UnreadCountResp {
    pub unread_count: i64,
}

async fn unread_count(claims: Claims, state: State<AppState>) -> WebResult<UnreadCountResp> {
    let unread_count = notification::unread_count(&state.sql_pool, claims.uid()).await?;
    ok!(UnreadCountResp { unread_count })
}

/// @since 261005
#[derive(Debug, Serialize, Deserialize)]
pub struct MarkReadReq {
    pub notification_id: String,
}

/// @since 261005
#[derive(Debug, Serialize, Deserialize)]
pub struct MarkReadResp {
    pub notification_id: Uuid,
    /// The first time it was marked read.
    pub read_time: DateTime<Utc>,
    /// Use this to replace the badge instead of counting locally.
    pub unread_count: i64,
}

async fn mark_read(claims: Claims, state: State<AppState>, req: Json<MarkReadReq>) -> WebResult<MarkReadResp> {
    let id = parse_id(&req.notification_id)?;
    match notification::mark_read(&state.sql_pool, claims.uid(), id).await? {
        Some((read_time, unread_count)) => ok!(MarkReadResp { notification_id: id, read_time, unread_count }),
        None => err!("notification_unavailable", "The notification does not exist or has expired"),
    }
}

/// @since 261005
#[derive(Debug, Serialize, Deserialize)]
pub struct MarkAllReadResp {
    pub marked_count: u64,
    /// May be non-zero if a notification arrived meanwhile.
    pub unread_count: i64,
}

async fn mark_all_read(claims: Claims, state: State<AppState>) -> WebResult<MarkAllReadResp> {
    let (marked_count, unread_count) = notification::mark_all_read(&state.sql_pool, claims.uid()).await?;
    ok!(MarkAllReadResp { marked_count, unread_count })
}
