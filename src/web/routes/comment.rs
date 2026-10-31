use crate::db::comment::CommentId;
use crate::service::comment::{self, CommentError, CommentItem, CommentTarget, CommentThread};
use crate::service::errors::ServiceError;
use crate::web::jwt::{Claims, OptionalClaims};
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::state::AppState;
use crate::{common, err, ok};
use async_backtrace::framed;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Comments on songs. See [crate::service::comment].
/// @since 261021
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/list", get(list))
        .route("/hot", get(hot))
        .route("/replies", get(replies))
        .route("/create", post(create))
        .route("/delete", post(delete))
        .route("/like", post(like))
        .route("/unlike", post(unlike))
}

fn parse_target(target_type: &str) -> Result<CommentTarget, WebError<CommonError>> {
    CommentTarget::parse(target_type).ok_or_else(|| common!("invalid_target_type", "Comments can't be posted on {target_type}"))
}

/// @since 261021
#[derive(Debug, Serialize, Deserialize)]
pub struct ListReq {
    /// `song`
    pub target_type: String,
    /// The song id, not its display id.
    pub target_id: i64,
    /// `newest` (the default) or `oldest`.
    pub order: Option<String>,
    /// `comment.create_time` of the previous page's last thread. Omit both for the first page.
    pub after_time: Option<DateTime<Utc>>,
    /// `comment.id` of the previous page's last thread.
    pub after_comment_id: Option<CommentId>,
    /// 1..=50, defaults to 20.
    pub limit: Option<i64>,
}

/// @since 261021
#[derive(Debug, Serialize, Deserialize)]
pub struct ListResp {
    /// Visible comments and replies on the target, for showing next to it.
    pub total: i64,
    pub threads: Vec<CommentThread>,
    pub has_more: bool,
}

/// Top-level comments on a target by time, each with its first few replies.
#[framed]
async fn list(claims: OptionalClaims, state: State<AppState>, req: Query<ListReq>) -> WebResult<ListResp> {
    let target = parse_target(&req.target_type)?;
    let oldest_first = match req.order.as_deref() {
        None | Some("newest") => false,
        Some("oldest") => true,
        Some(_) => err!("invalid_order", "Order must be `newest` or `oldest`"),
    };
    let after = match (req.after_time, req.after_comment_id) {
        (Some(time), Some(id)) => Some((time, id)),
        (None, None) => None,
        _ => err!("invalid_cursor", "after_time and after_comment_id must be given together"),
    };
    let page = comment::list(
        &state.sql_pool,
        claims.0.map(|x| x.uid()),
        target,
        req.target_id,
        oldest_first,
        after,
        req.limit.unwrap_or(20).clamp(1, 50),
    ).await?;
    ok!(ListResp { total: page.total, threads: page.threads, has_more: page.has_more })
}

/// @since 261021
#[derive(Debug, Serialize, Deserialize)]
pub struct HotReq {
    /// `song`
    pub target_type: String,
    /// The song id, not its display id.
    pub target_id: i64,
}

/// @since 261021
#[derive(Debug, Serialize, Deserialize)]
pub struct HotResp {
    /// Most liked first. Also in `/comment/list`, in their place by time.
    pub threads: Vec<CommentThread>,
}

/// The most liked top-level comments on a target, for showing above the rest: up to 10, each with
/// at least 3 likes.
#[framed]
async fn hot(claims: OptionalClaims, state: State<AppState>, req: Query<HotReq>) -> WebResult<HotResp> {
    let target = parse_target(&req.target_type)?;
    let threads = comment::list_hot(&state.sql_pool, claims.0.map(|x| x.uid()), target, req.target_id).await?;
    ok!(HotResp { threads })
}

/// @since 261021
#[derive(Debug, Serialize, Deserialize)]
pub struct RepliesReq {
    pub root_id: CommentId,
    /// From 0.
    pub page_index: Option<i64>,
    /// 1..=50, defaults to 20.
    pub page_size: Option<i64>,
}

/// @since 261021
#[derive(Debug, Serialize, Deserialize)]
pub struct RepliesResp {
    pub total: i64,
    /// Oldest first.
    pub replies: Vec<CommentItem>,
}

/// Replies under a top-level comment.
#[framed]
async fn replies(claims: OptionalClaims, state: State<AppState>, req: Query<RepliesReq>) -> WebResult<RepliesResp> {
    let page = comment::list_replies(
        &state.sql_pool,
        claims.0.map(|x| x.uid()),
        req.root_id,
        req.page_index.unwrap_or(0).max(0),
        req.page_size.unwrap_or(20).clamp(1, 50),
    ).await?;
    ok!(RepliesResp { total: page.total, replies: page.replies })
}

/// @since 261021
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateReq {
    /// `song`
    pub target_type: String,
    pub target_id: i64,
    /// Up to 500 characters, trimmed.
    pub content: String,
    /// The comment to reply to, a top-level comment or a reply under one; omit to comment on the
    /// target.
    pub reply_to_id: Option<CommentId>,
}

/// @since 261021
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateResp {
    /// The new comment; `None` if it couldn't be checked now and waits to be checked again. It's
    /// seen by no one, its author included, until it passes.
    pub comment: Option<CommentItem>,
}

/// Posts a comment or a reply. A comment the check blocks isn't saved and errors with
/// `comment_rejected`.
#[framed]
async fn create(claims: Claims, state: State<AppState>, req: Json<CreateReq>) -> WebResult<CreateResp> {
    let target = parse_target(&req.target_type)?;
    let comment = comment::create(
        &state.sql_pool,
        &state.red_lock,
        state.content_checker.as_deref(),
        claims.uid(),
        target,
        req.target_id,
        &req.content,
        req.reply_to_id,
    ).await?;
    ok!(CreateResp { comment })
}

/// @since 261021
#[derive(Debug, Serialize, Deserialize)]
pub struct CommentIdReq {
    pub comment_id: CommentId,
}

/// Deletes a comment, as its author or as the owner of what it's on.
#[framed]
async fn delete(claims: Claims, state: State<AppState>, req: Json<CommentIdReq>) -> WebResult<()> {
    comment::delete(&state.sql_pool, claims.uid(), req.comment_id).await?;
    ok!(())
}

#[framed]
async fn like(claims: Claims, state: State<AppState>, req: Json<CommentIdReq>) -> WebResult<()> {
    comment::like(&state.sql_pool, claims.uid(), req.comment_id).await?;
    ok!(())
}

#[framed]
async fn unlike(claims: Claims, state: State<AppState>, req: Json<CommentIdReq>) -> WebResult<()> {
    comment::unlike(&state.sql_pool, claims.uid(), req.comment_id).await?;
    ok!(())
}

impl From<ServiceError<CommentError>> for WebError<CommonError> {
    fn from(err: ServiceError<CommentError>) -> Self {
        match err {
            ServiceError::BusinessError(e) => {
                let code = match e {
                    CommentError::TargetNotFound => "target_not_found",
                    CommentError::CommentNotFound => "comment_not_found",
                    CommentError::ContentEmpty => "comment_empty",
                    CommentError::ContentTooLong => "comment_too_long",
                    CommentError::Rejected => "comment_rejected",
                    CommentError::RateLimited => "rate_limited",
                    CommentError::UserBanned => "user_banned",
                    CommentError::PermissionDenied => "permission_denied",
                };
                common!(code, "{}", e)
            }
            ServiceError::Other(e) => WebError::Internal(e),
        }
    }
}
