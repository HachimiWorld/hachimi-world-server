use crate::db::report::{ReportCase, STATUS_PENDING, STATUS_RESOLVED};
use crate::db::user::{IUserDao, UserDao};
use crate::service::committee;
use crate::service::errors::ServiceError;
use crate::service::report::{self, CaseSummary, Decision, ReportError, Verdict};
use crate::service::ugc::{UgcKind, UgcTarget};
use crate::web::jwt::Claims;
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::state::AppState;
use crate::{common, err, ok};
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;

/// @since 261008
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/submit", post(submit))
        .route("/queue", get(queue))
        .route("/case", get(case))
        .route("/resolve", post(resolve))
        .route("/hidden_reason", get(hidden_reason))
}

/// @since 261008
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBrief {
    pub uid: i64,
    pub username: String,
    pub avatar_url: Option<String>,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitReq {
    /// `song`, `playlist` or `user`
    pub target_type: String,
    pub target_id: i64,
    /// `spam`, `abuse`, `illegal`, `nsfw`, `copyright` or `other`
    pub reason: String,
    /// Required for `other`, up to 500 characters.
    pub detail: Option<String>,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitResp {
    pub report_id: i64,
    /// The target was reviewed before and further reports on it are ignored. The report is
    /// recorded anyway.
    pub already_reviewed: bool,
}

async fn submit(claims: Claims, state: State<AppState>, req: Json<SubmitReq>) -> WebResult<SubmitResp> {
    let Some(kind) = UgcKind::parse(&req.target_type) else {
        err!("invalid_target_type", "Unknown target type")
    };
    let result = report::submit(
        &state.sql_pool, &state.red_lock, claims.uid(), kind, req.target_id, &req.reason, req.detail.as_deref(),
    ).await?;
    ok!(SubmitResp { report_id: result.report_id, already_reviewed: result.already_reviewed })
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct QueueReq {
    /// `pending` (default) or `resolved`
    pub status: Option<String>,
    /// `last_report_time` of the previous page's last item. Omit both for the first page.
    pub before_time: Option<DateTime<Utc>>,
    /// `case_id` of the previous page's last item.
    pub before_id: Option<i64>,
    /// 1..=50, defaults to 20.
    pub limit: Option<i64>,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct QueueResp {
    pub items: Vec<CaseItem>,
    pub has_more: bool,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct CaseItem {
    pub case_id: i64,
    pub target_type: String,
    pub target_id: i64,
    /// `None` if the target was deleted.
    pub target: Option<TargetInfo>,
    /// `pending` or `resolved`
    pub status: String,
    pub pending_count: i32,
    /// Pending reports per reason, most first.
    pub reasons: Vec<ReasonCount>,
    pub last_report_time: DateTime<Utc>,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct TargetInfo {
    /// Song title, playlist name or username.
    pub title: String,
    pub cover_url: Option<String>,
    /// Song display id; empty for other kinds.
    pub display_id: String,
    pub owner: Option<UserBrief>,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct ReasonCount {
    pub reason: String,
    pub count: i64,
}

async fn queue(claims: Claims, state: State<AppState>, req: Query<QueueReq>) -> WebResult<QueueResp> {
    require_view(&state, claims.uid()).await?;
    let status = req.status.as_deref().unwrap_or(STATUS_PENDING);
    if status != STATUS_PENDING && status != STATUS_RESOLVED {
        err!("invalid_status", "Status must be `pending` or `resolved`")
    }
    let before = match (req.before_time, req.before_id) {
        (Some(time), Some(id)) => Some((time, id)),
        (None, None) => None,
        _ => err!("invalid_cursor", "`before_time` and `before_id` must be given together"),
    };
    let limit = req.limit.unwrap_or(20).clamp(1, 50);
    let (cases, has_more) = report::list_cases(&state.sql_pool, status, before, limit).await?;

    let uids: Vec<i64> = cases.iter().filter_map(|x| x.target.as_ref().map(|t| t.owner_uid)).collect();
    let users = load_users(&state.sql_pool, &uids).await?;
    let items = cases.into_iter().map(|x| case_item(x, &users)).collect();
    ok!(QueueResp { items, has_more })
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct CaseReq {
    pub target_type: String,
    pub target_id: i64,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct CaseResp {
    pub case: CaseItem,
    /// Oldest first, up to 200.
    pub pending_reports: Vec<ReportItem>,
    /// Newest first, up to 50.
    pub actions: Vec<ActionItem>,
    /// Verdicts a contributor can choose now; empty when there is nothing to decide on.
    pub verdicts: Vec<String>,
    /// Content actions a contributor can take now, each with the verdict it goes with.
    /// @since 261008
    pub content_actions: Vec<ContentActionOption>,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct ContentActionOption {
    /// Declared by the target's kind: `hide` and `restore` for songs and playlists, `reset_avatar`,
    /// `reset_bio` and `reset_username` for users.
    pub action: String,
    /// The verdict it can be taken with.
    pub verdict: String,
    /// Acts against the content, so the owner must be told why (`author_reason`).
    pub penalty: bool,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct ReportItem {
    pub report_id: i64,
    pub reporter: Option<UserBrief>,
    pub reason: String,
    pub detail: Option<String>,
    pub create_time: DateTime<Utc>,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct ActionItem {
    pub action_id: i64,
    pub operator: Option<UserBrief>,
    /// `agree`, `disagree` or `ignore`
    pub verdict: String,
    /// Internal, seen only by the committee and contributors.
    pub note: Option<String>,
    pub ignore_reports: bool,
    /// What it did to the content.
    /// @since 261008
    pub content_actions: Vec<String>,
    /// Why, as told to the owner.
    /// @since 261008
    pub author_reason: Option<String>,
    pub create_time: DateTime<Utc>,
}

async fn case(claims: Claims, state: State<AppState>, req: Query<CaseReq>) -> WebResult<CaseResp> {
    require_view(&state, claims.uid()).await?;
    let Some(kind) = UgcKind::parse(&req.target_type) else {
        err!("invalid_target_type", "Unknown target type")
    };
    let Some(detail) = report::get_case(&state.sql_pool, kind, req.target_id).await? else {
        err!("case_not_found", "This target has no reports")
    };

    let mut uids: Vec<i64> = detail.pending_reports.iter().map(|x| x.reporter_uid).collect();
    uids.extend(detail.actions.iter().map(|x| x.operator_uid));
    uids.extend(detail.summary.target.as_ref().map(|x| x.owner_uid));
    let users = load_users(&state.sql_pool, &uids).await?;

    let options = report::decision_options(kind, &detail.summary.case, detail.summary.target.as_ref());
    let verdicts = options.verdicts.iter().map(|x| x.as_str().to_string()).collect();
    let content_actions = options.content_actions.iter()
        .map(|(action, verdict)| ContentActionOption {
            action: action.id.to_string(),
            verdict: verdict.as_str().to_string(),
            penalty: action.penalty,
        })
        .collect();
    let pending_reports = detail.pending_reports.into_iter().map(|x| ReportItem {
        report_id: x.id,
        reporter: users.get(&x.reporter_uid).cloned(),
        reason: x.reason,
        detail: x.detail,
        create_time: x.create_time,
    }).collect();
    let actions = detail.actions.into_iter().map(|x| ActionItem {
        action_id: x.id,
        operator: users.get(&x.operator_uid).cloned(),
        verdict: x.verdict,
        note: x.note,
        ignore_reports: x.ignore_reports,
        content_actions: x.content_actions,
        author_reason: x.author_reason,
        create_time: x.create_time,
    }).collect();
    ok!(CaseResp {
        case: case_item(detail.summary, &users),
        pending_reports,
        actions,
        verdicts,
        content_actions,
    })
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct ResolveReq {
    pub target_type: String,
    pub target_id: i64,
    /// One of the case's `verdicts`.
    pub verdict: String,
    /// Internal, up to 1000 characters.
    pub note: Option<String>,
    /// From the case's `content_actions` for this verdict.
    /// @since 261008
    #[serde(default)]
    pub content_actions: Vec<String>,
    /// Shown to the owner, up to 500 characters; required for penalties.
    /// @since 261008
    pub author_reason: Option<String>,
    /// Record further reports without reopening the case.
    #[serde(default)]
    pub ignore_reports: bool,
    /// The last pending report the operator saw; later ones stay pending. 0 if there was none.
    pub up_to_report_id: i64,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct ResolveResp {
    pub action_id: i64,
    /// `pending` if reports arrived after `up_to_report_id`.
    pub status: String,
    pub pending_count: i32,
}

async fn resolve(claims: Claims, state: State<AppState>, req: Json<ResolveReq>) -> WebResult<ResolveResp> {
    let access = committee::access(&state, claims.uid()).await?;
    if !access.can_resolve {
        err!("permission_denied", "Only contributors can resolve reports")
    }
    let Some(kind) = UgcKind::parse(&req.target_type) else {
        err!("invalid_target_type", "Unknown target type")
    };
    let Some(verdict) = Verdict::parse(&req.verdict) else {
        err!("invalid_verdict", "Unknown verdict")
    };
    let result = report::resolve(&state, claims.uid(), kind, req.target_id, Decision {
        verdict,
        content_actions: req.content_actions.clone(),
        author_reason: req.author_reason.as_deref(),
        note: req.note.as_deref(),
        ignore_reports: req.ignore_reports,
        up_to_report_id: req.up_to_report_id,
    }).await?;
    ok!(ResolveResp { action_id: result.action_id, status: result.status, pending_count: result.pending_count })
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct HiddenReasonReq {
    pub target_type: String,
    pub target_id: i64,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct HiddenReasonResp {
    /// Whether the user's song or playlist is hidden.
    pub hidden: bool,
    /// Why, if it is.
    pub reason: Option<String>,
    pub hide_time: Option<DateTime<Utc>>,
}

/// For the owner of a song or playlist: whether it's hidden, and why.
async fn hidden_reason(claims: Claims, state: State<AppState>, req: Query<HiddenReasonReq>) -> WebResult<HiddenReasonResp> {
    let Some(kind) = UgcKind::parse(&req.target_type) else {
        err!("invalid_target_type", "Unknown target type")
    };
    let notice = report::hidden_reason(&state.sql_pool, claims.uid(), kind, req.target_id).await?;
    ok!(HiddenReasonResp {
        hidden: notice.is_some(),
        reason: notice.as_ref().and_then(|x| x.reason.clone()),
        hide_time: notice.map(|x| x.hide_time),
    })
}

async fn require_view(state: &AppState, uid: i64) -> Result<(), WebError<CommonError>> {
    if committee::access(state, uid).await?.can_view {
        Ok(())
    } else {
        Err(common!("permission_denied", "Only the committee and contributors can view reports"))
    }
}

pub async fn load_users(pool: &PgPool, uids: &[i64]) -> anyhow::Result<HashMap<i64, UserBrief>> {
    let mut uids = uids.to_vec();
    uids.sort_unstable();
    uids.dedup();
    Ok(UserDao::list_by_ids(pool, &uids).await?
        .into_iter()
        .map(|x| (x.id, UserBrief { uid: x.id, username: x.username, avatar_url: x.avatar_url }))
        .collect())
}

fn case_item(x: CaseSummary, users: &HashMap<i64, UserBrief>) -> CaseItem {
    let CaseSummary { case, target, reasons } = x;
    let ReportCase { id, target_type, target_id, status, pending_count, last_report_time, .. } = case;
    CaseItem {
        case_id: id,
        target_type,
        target_id,
        target: target.map(|t| target_info(t, users)),
        status,
        pending_count,
        reasons: reasons.into_iter().map(|(reason, count)| ReasonCount { reason, count }).collect(),
        last_report_time,
    }
}

fn target_info(t: UgcTarget, users: &HashMap<i64, UserBrief>) -> TargetInfo {
    TargetInfo {
        owner: users.get(&t.owner_uid).cloned(),
        title: t.title,
        cover_url: t.cover_url,
        display_id: t.display_id,
    }
}

impl From<ServiceError<ReportError>> for WebError<CommonError> {
    fn from(err: ServiceError<ReportError>) -> Self {
        match err {
            ServiceError::BusinessError(e) => {
                let code = match e {
                    ReportError::TargetNotFound => "target_not_found",
                    ReportError::CannotReportSelf => "cannot_report_self",
                    ReportError::InvalidReason => "invalid_reason",
                    ReportError::DetailRequired => "detail_required",
                    ReportError::DetailTooLong => "detail_too_long",
                    ReportError::AlreadyReported => "already_reported",
                    ReportError::RateLimited => "rate_limited",
                    ReportError::UserBanned => "user_banned",
                    ReportError::CaseNotFound => "case_not_found",
                    ReportError::InvalidVerdict => "invalid_verdict",
                    ReportError::NoteTooLong => "note_too_long",
                    ReportError::InvalidContentAction => "invalid_content_action",
                    ReportError::ContentActionRequired => "content_action_required",
                    ReportError::AuthorReasonRequired => "author_reason_required",
                    ReportError::AuthorReasonTooLong => "author_reason_too_long",
                };
                common!(code, "{}", e)
            }
            ServiceError::Other(e) => WebError::Internal(e),
        }
    }
}
