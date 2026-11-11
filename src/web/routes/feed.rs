use crate::service::feed::{self, Cursor, FeedItem};
use crate::web::jwt::OptionalClaims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{err, ok};
use async_backtrace::framed;
use axum::extract::{Query, State};
use aide::axum::routing::get_with;
use crate::web::routes::since;
use aide::axum::ApiRouter;
use serde::{Deserialize, Serialize};

/// Streams that mix kinds of content. See [crate::service::feed].
/// @since 261106
pub fn router() -> ApiRouter<AppState> {
    ApiRouter::new()
        .api_route("/square", get_with(square, since("261106")))
}

/// @since 261106
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PageReq {
    /// `next_cursor` of the previous page; omit for the first page.
    pub cursor: Option<String>,
    /// 1..=50, defaults to 20.
    pub limit: Option<i64>,
}

/// @since 261106
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PageResp {
    /// Newest first. Skip kinds you don't know.
    pub items: Vec<FeedItem>,
    /// `None` when there's no more.
    pub next_cursor: Option<String>,
}

/// The square: everyone's moments, newest first.
#[framed]
async fn square(claims: OptionalClaims, state: State<AppState>, req: Query<PageReq>) -> WebResult<PageResp> {
    let cursor = match req.cursor.as_deref() {
        None => None,
        Some(x) => match Cursor::decode(x) {
            Some(x) => Some(x),
            None => err!("invalid_cursor", "Invalid cursor"),
        },
    };
    let page = feed::square(&state, claims.0.map(|x| x.uid()), cursor, req.limit.unwrap_or(20).clamp(1, 50)).await?;
    ok!(PageResp { items: page.items, next_cursor: page.next_cursor })
}
