//! Reports and report cases. Each reported target has one case, reused forever: reporting a
//! resolved target reopens its case, unless the last action chose to ignore further reports.
//! See `docs/specs/governance/README.md`.

use crate::db::report::{ModerationAction, ModerationActionDao, Report, ReportCase, ReportCaseDao, ReportDao, STATUS_PENDING, STATUS_RESOLVED};
use crate::db::user::UserDao;
use crate::db::CrudDao;
use crate::service::errors::{ServiceError, ServiceResult};
use crate::service::notification::{send_notification, to_plain_text, NewNotification};
use crate::service::ugc::{ContentAction, UgcAdapter, UgcKind, UgcTarget};
use crate::web::state::AppState;
use crate::util::redlock::{RedLock, RedLockGuard};
use anyhow::anyhow;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use sqlx::PgPool;
use std::collections::HashSet;
use uuid::Uuid;

pub const REASONS: &[&str] = &["spam", "abuse", "illegal", "nsfw", "copyright", "other"];
const DETAIL_MAX_CHARS: usize = 500;
const NOTE_MAX_CHARS: usize = 1000;
const AUTHOR_REASON_MAX_CHARS: usize = 500;
const REPORTS_PER_HOUR: i64 = 20;
/// Pending reports shown in a case's detail.
const PENDING_REPORTS_SHOWN: i64 = 200;
/// Past actions shown in a case's detail.
const ACTIONS_SHOWN: i64 = 50;

/// Whether the reports were right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Agree,
    Disagree,
    Ignore,
}

impl Verdict {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "agree" => Some(Self::Agree),
            "disagree" => Some(Self::Disagree),
            "ignore" => Some(Self::Ignore),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agree => "agree",
            Self::Disagree => "disagree",
            Self::Ignore => "ignore",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("target not found")]
    TargetNotFound,
    #[error("cannot report yourself or your own content")]
    CannotReportSelf,
    #[error("invalid reason")]
    InvalidReason,
    #[error("detail is required for reason `other`")]
    DetailRequired,
    #[error("detail is too long")]
    DetailTooLong,
    #[error("you already reported this and it hasn't been handled yet")]
    AlreadyReported,
    #[error("too many reports, try again later")]
    RateLimited,
    #[error("banned users can't report")]
    UserBanned,
    #[error("case not found")]
    CaseNotFound,
    /// Not one of [available_verdicts].
    #[error("verdict not available")]
    InvalidVerdict,
    #[error("note is too long")]
    NoteTooLong,
    /// Not one of [DecisionOptions::content_actions] for the verdict.
    #[error("content action not available")]
    InvalidContentAction,
    /// Upholding reports must do something, unless the content is already hidden.
    #[error("choose a content action")]
    ContentActionRequired,
    #[error("a reason for the owner is required")]
    AuthorReasonRequired,
    #[error("the reason for the owner is too long")]
    AuthorReasonTooLong,
}

pub struct SubmitResult {
    pub report_id: i64,
    /// The target was reviewed before and further reports are ignored; the report is recorded
    /// without reopening the case.
    pub already_reviewed: bool,
}

pub struct CaseSummary {
    pub case: ReportCase,
    /// `None` if the target was deleted.
    pub target: Option<UgcTarget>,
    /// Pending reports per reason, most first.
    pub reasons: Vec<(String, i64)>,
}

pub struct CaseDetail {
    pub summary: CaseSummary,
    pub pending_reports: Vec<Report>,
    /// Newest first.
    pub actions: Vec<ModerationAction>,
}

pub struct ResolveResult {
    pub action_id: i64,
    /// `pending` if reports arrived after `up_to_report_id`.
    pub status: String,
    pub pending_count: i32,
}

fn business<T>(e: ReportError) -> ServiceResult<T, ReportError> {
    Err(ServiceError::BusinessError(e))
}

async fn lock_case(red_lock: &RedLock, kind: UgcKind, target_id: i64) -> anyhow::Result<RedLockGuard> {
    red_lock.lock_with_timeout(&format!("report:case:{}:{}", kind.as_str(), target_id), std::time::Duration::from_secs(10)).await?
        .ok_or_else(|| anyhow!("Can't get the report case lock"))
}

pub async fn submit(
    pool: &PgPool,
    red_lock: &RedLock,
    reporter_uid: i64,
    kind: UgcKind,
    target_id: i64,
    reason: &str,
    detail: Option<&str>,
) -> ServiceResult<SubmitResult, ReportError> {
    if !REASONS.contains(&reason) {
        return business(ReportError::InvalidReason);
    }
    let detail = detail.map(str::trim).filter(|x| !x.is_empty());
    if detail.is_some_and(|x| x.chars().count() > DETAIL_MAX_CHARS) {
        return business(ReportError::DetailTooLong);
    }
    if reason == "other" && detail.is_none() {
        return business(ReportError::DetailRequired);
    }

    let reporter = UserDao::get_by_id(pool, reporter_uid).await?.ok_or_else(|| anyhow!("Reporter {reporter_uid} not found"))?;
    if reporter.is_banned {
        return business(ReportError::UserBanned);
    }
    let Some(target) = kind.load(pool, target_id).await? else {
        return business(ReportError::TargetNotFound);
    };
    if target.owner_uid == reporter_uid {
        return business(ReportError::CannotReportSelf);
    }
    if !target.is_public {
        return business(ReportError::TargetNotFound);
    }
    let now = Utc::now().trunc_subsecs(6);
    if ReportDao::count_by_reporter_since(pool, reporter_uid, now - Duration::hours(1)).await? >= REPORTS_PER_HOUR {
        return business(ReportError::RateLimited);
    }

    let _lock = lock_case(red_lock, kind, target_id).await?;
    let mut tx = pool.begin().await?;
    let mut report = Report {
        id: 0,
        case_id: 0,
        reporter_uid,
        reason: reason.to_string(),
        detail: detail.map(str::to_string),
        action_id: None,
        create_time: now,
    };
    let mut already_reviewed = false;
    match ReportCaseDao::get_by_target(&mut *tx, kind.as_str(), target_id).await? {
        None => {
            report.case_id = ReportCaseDao::insert(&mut *tx, &ReportCase {
                id: 0,
                target_type: kind.as_str().to_string(),
                target_id,
                target_owner_uid: target.owner_uid,
                status: STATUS_PENDING.to_string(),
                ignore_reports: false,
                pending_count: 1,
                last_report_time: now,
                create_time: now,
                update_time: now,
            }).await?;
        }
        Some(mut case) => {
            let previous = ReportDao::list_by_case_and_reporter(&mut *tx, case.id, reporter_uid).await?;
            if previous.iter().any(|x| x.action_id.is_none()) {
                return business(ReportError::AlreadyReported);
            }
            report.case_id = case.id;
            let last_action = if case.ignore_reports {
                ModerationActionDao::list_by_case(&mut *tx, case.id, 1).await?.pop()
            } else {
                None
            };
            if let Some(action) = last_action {
                // Recorded as handled by the action that chose to ignore further reports
                report.action_id = Some(action.id);
                already_reviewed = true;
            } else {
                case.target_owner_uid = target.owner_uid;
                case.status = STATUS_PENDING.to_string();
                case.pending_count += 1;
                case.last_report_time = now;
                case.update_time = now;
                ReportCaseDao::update(&mut *tx, &case).await?;
            }
        }
    }
    let report_id = ReportDao::insert(&mut *tx, &report).await?;
    tx.commit().await?;
    Ok(SubmitResult { report_id, already_reviewed })
}

/// Cases with `status`, most recently reported first.
pub async fn list_cases(
    pool: &PgPool,
    status: &str,
    before: Option<(DateTime<Utc>, i64)>,
    limit: i64,
) -> sqlx::Result<(Vec<CaseSummary>, bool)> {
    let mut cases = ReportCaseDao::list_by_status(pool, status, before, limit + 1).await?;
    let has_more = cases.len() as i64 > limit;
    cases.truncate(limit as usize);

    let case_ids: Vec<i64> = cases.iter().map(|x| x.id).collect();
    let reason_counts = ReportDao::count_pending_reasons(pool, &case_ids).await?;
    let mut items = Vec::with_capacity(cases.len());
    for case in cases {
        let target = load_target(pool, &case).await?;
        let reasons = reason_counts.iter()
            .filter(|x| x.case_id == case.id)
            .map(|x| (x.reason.clone(), x.count))
            .collect();
        items.push(CaseSummary { case, target, reasons });
    }
    Ok((items, has_more))
}

pub async fn get_case(pool: &PgPool, kind: UgcKind, target_id: i64) -> sqlx::Result<Option<CaseDetail>> {
    let Some(case) = ReportCaseDao::get_by_target(pool, kind.as_str(), target_id).await? else {
        return Ok(None);
    };
    let target = kind.load(pool, target_id).await?;
    let reasons = ReportDao::count_pending_reasons(pool, &[case.id]).await?
        .into_iter()
        .map(|x| (x.reason, x.count))
        .collect();
    let pending_reports = ReportDao::list_pending(pool, case.id, PENDING_REPORTS_SHOWN).await?;
    let actions = ModerationActionDao::list_by_case(pool, case.id, ACTIONS_SHOWN).await?;
    Ok(Some(CaseDetail { summary: CaseSummary { case, target, reasons }, pending_reports, actions }))
}

/// What a contributor can decide on a case now.
pub struct DecisionOptions {
    pub verdicts: Vec<Verdict>,
    /// Content actions and the verdict each goes with: penalties with `agree`, actions undoing an
    /// earlier penalty with `disagree`.
    pub content_actions: Vec<(ContentAction, Verdict)>,
}

/// With pending reports, they can be upheld (when there's a penalty to take or the target is
/// already hidden), rejected or ignored. Without, an earlier penalty can still be undone, and a
/// case that ignores reports can take them again. Which content actions exist is up to the kind.
pub fn decision_options(kind: UgcKind, case: &ReportCase, target: Option<&UgcTarget>) -> DecisionOptions {
    let actions = target.map(|x| kind.content_actions(x)).unwrap_or_default();
    let (penalties, reversals): (Vec<ContentAction>, Vec<ContentAction>) = actions.iter().partition(|x| x.penalty);
    let hidden = target.is_some_and(|x| x.is_hidden);

    let mut verdicts = vec![];
    if case.pending_count > 0 {
        if !penalties.is_empty() || hidden {
            verdicts.push(Verdict::Agree);
        }
        verdicts.extend([Verdict::Disagree, Verdict::Ignore]);
    } else {
        if !reversals.is_empty() {
            verdicts.push(Verdict::Disagree);
        }
        if case.ignore_reports {
            verdicts.push(Verdict::Ignore);
        }
    }

    let mut content_actions = vec![];
    if verdicts.contains(&Verdict::Agree) {
        content_actions.extend(penalties.iter().map(|x| (*x, Verdict::Agree)));
    }
    if verdicts.contains(&Verdict::Disagree) {
        content_actions.extend(reversals.iter().map(|x| (*x, Verdict::Disagree)));
    }
    DecisionOptions { verdicts, content_actions }
}

/// A contributor's decision on a case.
pub struct Decision<'a> {
    pub verdict: Verdict,
    /// Ids from [DecisionOptions::content_actions] for the verdict.
    pub content_actions: Vec<String>,
    /// Shown to the owner when content actions are taken; required for penalties.
    pub author_reason: Option<&'a str>,
    /// Internal.
    pub note: Option<&'a str>,
    pub ignore_reports: bool,
    /// The last pending report the operator saw; later ones stay pending.
    pub up_to_report_id: i64,
}

/// Decides on the pending reports of a case up to `up_to_report_id`, takes the content actions,
/// and tells the reporters and the owner.
pub async fn resolve(
    state: &AppState,
    operator_uid: i64,
    kind: UgcKind,
    target_id: i64,
    decision: Decision<'_>,
) -> ServiceResult<ResolveResult, ReportError> {
    let note = decision.note.map(str::trim).filter(|x| !x.is_empty());
    if note.is_some_and(|x| x.chars().count() > NOTE_MAX_CHARS) {
        return business(ReportError::NoteTooLong);
    }
    let author_reason = decision.author_reason.map(str::trim).filter(|x| !x.is_empty());
    if author_reason.is_some_and(|x| x.chars().count() > AUTHOR_REASON_MAX_CHARS) {
        return business(ReportError::AuthorReasonTooLong);
    }
    let verdict = decision.verdict;

    let pool = &state.sql_pool;
    let _lock = lock_case(&state.red_lock, kind, target_id).await?;
    let mut tx = pool.begin().await?;
    let Some(mut case) = ReportCaseDao::get_by_target(&mut *tx, kind.as_str(), target_id).await? else {
        return business(ReportError::CaseNotFound);
    };
    let target = kind.load(pool, target_id).await?;
    let options = decision_options(kind, &case, target.as_ref());
    if !options.verdicts.contains(&verdict) {
        return business(ReportError::InvalidVerdict);
    }
    let mut actions: Vec<ContentAction> = vec![];
    for id in &decision.content_actions {
        let Some((action, _)) = options.content_actions.iter().find(|(x, v)| x.id == id && *v == verdict) else {
            return business(ReportError::InvalidContentAction);
        };
        if !actions.contains(action) {
            actions.push(*action);
        }
    }
    let already_hidden = target.as_ref().is_some_and(|x| x.is_hidden);
    if verdict == Verdict::Agree && actions.is_empty() && !already_hidden {
        return business(ReportError::ContentActionRequired);
    }
    if actions.iter().any(|x| x.penalty) && author_reason.is_none() {
        return business(ReportError::AuthorReasonRequired);
    }

    let now = Utc::now().trunc_subsecs(6);
    let action_id = ModerationActionDao::insert(&mut *tx, &ModerationAction {
        id: 0,
        case_id: case.id,
        operator_uid,
        verdict: verdict.as_str().to_string(),
        note: note.map(str::to_string),
        ignore_reports: decision.ignore_reports,
        up_to_report_id: decision.up_to_report_id,
        target_snapshot: serde_json::to_value(&target)?,
        content_actions: actions.iter().map(|x| x.id.to_string()).collect(),
        author_reason: author_reason.map(str::to_string),
        create_time: now,
    }).await?;
    if !actions.is_empty() {
        kind.apply(&mut tx, target_id, &actions).await?;
    }
    let reporters = ReportDao::resolve_up_to(&mut *tx, case.id, decision.up_to_report_id, action_id).await?;

    let pending_count = ReportDao::count_pending(&mut *tx, case.id).await? as i32;
    case.pending_count = pending_count;
    case.status = if pending_count == 0 { STATUS_RESOLVED } else { STATUS_PENDING }.to_string();
    case.ignore_reports = decision.ignore_reports;
    case.update_time = now;
    ReportCaseDao::update(&mut *tx, &case).await?;

    let mut notified = HashSet::new();
    for uid in reporters {
        if notified.insert(uid) {
            send_notification(&mut tx, report_resolved_notification(uid, kind, target.as_ref(), verdict, now)).await?;
        }
    }
    if let Some(target) = &target
        && let Some(n) = owner_notification(kind, target_id, target, &actions, author_reason, now)
    {
        send_notification(&mut tx, n).await?;
    }
    tx.commit().await?;

    if !actions.is_empty() {
        kind.refresh(state, target_id).await;
    }
    Ok(ResolveResult { action_id, status: case.status, pending_count })
}

/// Why a target the user owns is hidden.
pub struct HiddenReason {
    pub reason: Option<String>,
    pub hide_time: DateTime<Utc>,
}

/// For the owner of a hidden song or playlist: the decision that hid it.
pub async fn hidden_reason(pool: &PgPool, uid: i64, kind: UgcKind, target_id: i64) -> sqlx::Result<Option<HiddenReason>> {
    let Some(target) = kind.load(pool, target_id).await? else {
        return Ok(None);
    };
    if target.owner_uid != uid || !target.is_hidden {
        return Ok(None);
    }
    let Some(case) = ReportCaseDao::get_by_target(pool, kind.as_str(), target_id).await? else {
        return Ok(None);
    };
    // The latest decision that penalized it
    let is_penalty = |id: &String| kind.actions().iter().any(|x| x.id == id && x.penalty);
    Ok(ModerationActionDao::list_by_case(pool, case.id, ACTIONS_SHOWN).await?
        .into_iter()
        .find(|x| x.content_actions.iter().any(is_penalty))
        .map(|x| HiddenReason { reason: x.author_reason, hide_time: x.create_time }))
}

async fn load_target(pool: &PgPool, case: &ReportCase) -> sqlx::Result<Option<UgcTarget>> {
    match UgcKind::parse(&case.target_type) {
        Some(kind) => kind.load(pool, case.target_id).await,
        None => Ok(None),
    }
}

fn report_resolved_notification(
    recipient_uid: i64,
    kind: UgcKind,
    target: Option<&UgcTarget>,
    verdict: Verdict,
    occurred_at: DateTime<Utc>,
) -> NewNotification {
    let target = kind.mention(target.map(|x| x.title.as_str()));
    let result = match verdict {
        Verdict::Agree => "已采取相应措施",
        Verdict::Disagree => "经审查未发现违规",
        Verdict::Ignore => "未采取措施",
    };
    NewNotification {
        id: Uuid::now_v7(),
        recipient_uid,
        notification_type: "governance.report_resolved",
        title: "举报已处理".to_string(),
        body: format!("你对{target}的举报已处理：{result}。感谢你的反馈。"),
        content_intent: None,
        occurred_at,
    }
}

/// Tells the owner what a decision did to their content. None if it did nothing.
fn owner_notification(
    kind: UgcKind,
    target_id: i64,
    target: &UgcTarget,
    actions: &[ContentAction],
    reason: Option<&str>,
    occurred_at: DateTime<Utc>,
) -> Option<NewNotification> {
    let reason_suffix = reason.map(|x| format!("\n\n原因：{}", to_plain_text(x))).unwrap_or_default();
    let message = kind.owner_message(target_id, target, actions, &reason_suffix)?;
    Some(NewNotification {
        id: Uuid::now_v7(),
        recipient_uid: target.owner_uid,
        notification_type: message.notification_type,
        title: message.title.to_string(),
        body: message.body,
        content_intent: Some(message.intent),
        occurred_at,
    })
}
