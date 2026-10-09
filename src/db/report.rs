//! Report cases, the reports in them, and the moderation actions taken on them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgExecutor;

macro_rules! query_report_cases {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            ReportCase, r#"
            SELECT
                id,
                target_type,
                target_id,
                target_owner_uid,
                status,
                ignore_reports,
                pending_count,
                last_report_time,
                create_time,
                update_time
            FROM report_cases
            "# + $extra,
            $($arg),*
        )
    };
}

macro_rules! query_reports {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            Report, r#"
            SELECT
                id,
                case_id,
                reporter_uid,
                reason,
                detail,
                action_id,
                create_time
            FROM reports
            "# + $extra,
            $($arg),*
        )
    };
}

macro_rules! query_moderation_actions {
    ($extra:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            ModerationAction, r#"
            SELECT
                id,
                case_id,
                operator_uid,
                verdict,
                note,
                ignore_reports,
                up_to_report_id,
                target_snapshot,
                content_actions,
                author_reason,
                create_time
            FROM moderation_actions
            "# + $extra,
            $($arg),*
        )
    };
}

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_RESOLVED: &str = "resolved";

/// One per reported target, reused forever.
/// @since 261008
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportCase {
    pub id: i64,
    /// `song`, `playlist` or `user`
    pub target_type: String,
    pub target_id: i64,
    pub target_owner_uid: i64,
    /// `pending` or `resolved`
    pub status: String,
    /// New reports are recorded but don't reopen the case.
    pub ignore_reports: bool,
    pub pending_count: i32,
    pub last_report_time: DateTime<Utc>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

/// @since 261008
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: i64,
    pub case_id: i64,
    pub reporter_uid: i64,
    pub reason: String,
    pub detail: Option<String>,
    /// The action that handled it; `None` while pending.
    pub action_id: Option<i64>,
    pub create_time: DateTime<Utc>,
}

/// @since 261008
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModerationAction {
    pub id: i64,
    pub case_id: i64,
    pub operator_uid: i64,
    /// `agree`, `disagree` or `ignore`
    pub verdict: String,
    pub note: Option<String>,
    pub ignore_reports: bool,
    pub up_to_report_id: i64,
    pub target_snapshot: serde_json::Value,
    /// What it did to the content, such as `hide` or `reset_avatar`.
    /// @since 261008
    pub content_actions: Vec<String>,
    /// Why, as told to the content's owner.
    /// @since 261008
    pub author_reason: Option<String>,
    pub create_time: DateTime<Utc>,
}

/// @since 261008
#[derive(Debug, Clone)]
pub struct ReasonCount {
    pub case_id: i64,
    pub reason: String,
    pub count: i64,
}

pub struct ReportCaseDao;

impl ReportCaseDao {
    pub async fn get_by_id<'e>(executor: impl PgExecutor<'e>, id: i64) -> sqlx::Result<Option<ReportCase>> {
        query_report_cases!("WHERE id = $1", id)
            .fetch_optional(executor).await
    }

    pub async fn get_by_target<'e>(executor: impl PgExecutor<'e>, target_type: &str, target_id: i64) -> sqlx::Result<Option<ReportCase>> {
        query_report_cases!("WHERE target_type = $1 AND target_id = $2 ORDER BY id LIMIT 1", target_type, target_id)
            .fetch_optional(executor).await
    }

    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &ReportCase) -> sqlx::Result<i64> {
        sqlx::query!(
            "INSERT INTO report_cases (target_type, target_id, target_owner_uid, status, ignore_reports, pending_count, last_report_time, create_time, update_time)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
            value.target_type, value.target_id, value.target_owner_uid, value.status, value.ignore_reports,
            value.pending_count, value.last_report_time, value.create_time, value.update_time
        ).fetch_one(executor).await.map(|x| x.id)
    }

    pub async fn update<'e>(executor: impl PgExecutor<'e>, value: &ReportCase) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE report_cases SET target_owner_uid = $2, status = $3, ignore_reports = $4, pending_count = $5, last_report_time = $6, update_time = $7
            WHERE id = $1",
            value.id, value.target_owner_uid, value.status, value.ignore_reports, value.pending_count, value.last_report_time, value.update_time
        ).execute(executor).await?;
        Ok(())
    }

    /// Cases with `status`, most recently reported first. Pass the last report time and id of the
    /// previous page's last case as `before`.
    pub async fn list_by_status<'e>(
        executor: impl PgExecutor<'e>,
        status: &str,
        before: Option<(DateTime<Utc>, i64)>,
        limit: i64,
    ) -> sqlx::Result<Vec<ReportCase>> {
        let (before_time, before_id) = before.unzip();
        query_report_cases!(
            "WHERE status = $1 AND ($2::timestamptz IS NULL OR (last_report_time, id) < ($2, $3::bigint))
            ORDER BY last_report_time DESC, id DESC
            LIMIT $4",
            status, before_time, before_id, limit
        ).fetch_all(executor).await
    }
}

pub struct ReportDao;

impl ReportDao {
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &Report) -> sqlx::Result<i64> {
        sqlx::query!(
            "INSERT INTO reports (case_id, reporter_uid, reason, detail, action_id, create_time) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
            value.case_id, value.reporter_uid, value.reason, value.detail, value.action_id, value.create_time
        ).fetch_one(executor).await.map(|x| x.id)
    }

    pub async fn list_by_case_and_reporter<'e>(executor: impl PgExecutor<'e>, case_id: i64, reporter_uid: i64) -> sqlx::Result<Vec<Report>> {
        query_reports!("WHERE case_id = $1 AND reporter_uid = $2 ORDER BY id", case_id, reporter_uid)
            .fetch_all(executor).await
    }

    pub async fn count_by_reporter_since<'e>(executor: impl PgExecutor<'e>, reporter_uid: i64, since: DateTime<Utc>) -> sqlx::Result<i64> {
        sqlx::query!(
            r#"SELECT COUNT(*) AS "count!" FROM reports WHERE reporter_uid = $1 AND create_time > $2"#,
            reporter_uid, since
        ).fetch_one(executor).await.map(|x| x.count)
    }

    /// Pending reports of a case, oldest first.
    pub async fn list_pending<'e>(executor: impl PgExecutor<'e>, case_id: i64, limit: i64) -> sqlx::Result<Vec<Report>> {
        query_reports!("WHERE case_id = $1 AND action_id IS NULL ORDER BY id LIMIT $2", case_id, limit)
            .fetch_all(executor).await
    }

    pub async fn count_pending<'e>(executor: impl PgExecutor<'e>, case_id: i64) -> sqlx::Result<i64> {
        sqlx::query!(
            r#"SELECT COUNT(*) AS "count!" FROM reports WHERE case_id = $1 AND action_id IS NULL"#,
            case_id
        ).fetch_one(executor).await.map(|x| x.count)
    }

    /// Pending reports per reason, for each of `case_ids`.
    pub async fn count_pending_reasons<'e>(executor: impl PgExecutor<'e>, case_ids: &[i64]) -> sqlx::Result<Vec<ReasonCount>> {
        sqlx::query_as!(
            ReasonCount,
            r#"SELECT case_id, reason, COUNT(*) AS "count!" FROM reports
            WHERE case_id = ANY($1) AND action_id IS NULL
            GROUP BY case_id, reason
            ORDER BY case_id, COUNT(*) DESC, reason"#,
            case_ids
        ).fetch_all(executor).await
    }

    /// Marks pending reports of a case up to `up_to_report_id` as handled by `action_id`, and
    /// returns their reporters.
    pub async fn resolve_up_to<'e>(executor: impl PgExecutor<'e>, case_id: i64, up_to_report_id: i64, action_id: i64) -> sqlx::Result<Vec<i64>> {
        sqlx::query!(
            "UPDATE reports SET action_id = $3 WHERE case_id = $1 AND id <= $2 AND action_id IS NULL RETURNING reporter_uid",
            case_id, up_to_report_id, action_id
        ).fetch_all(executor).await.map(|x| x.into_iter().map(|r| r.reporter_uid).collect())
    }
}

pub struct ModerationActionDao;

impl ModerationActionDao {
    pub async fn insert<'e>(executor: impl PgExecutor<'e>, value: &ModerationAction) -> sqlx::Result<i64> {
        sqlx::query!(
            "INSERT INTO moderation_actions (case_id, operator_uid, verdict, note, ignore_reports, up_to_report_id, target_snapshot, content_actions, author_reason, create_time)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
            value.case_id, value.operator_uid, value.verdict, value.note, value.ignore_reports,
            value.up_to_report_id, value.target_snapshot, &value.content_actions, value.author_reason, value.create_time
        ).fetch_one(executor).await.map(|x| x.id)
    }

    /// Actions on a case, newest first.
    pub async fn list_by_case<'e>(executor: impl PgExecutor<'e>, case_id: i64, limit: i64) -> sqlx::Result<Vec<ModerationAction>> {
        query_moderation_actions!("WHERE case_id = $1 ORDER BY id DESC LIMIT $2", case_id, limit)
            .fetch_all(executor).await
    }
}
