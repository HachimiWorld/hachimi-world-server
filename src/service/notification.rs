//! System notifications: results and status changes the server sends to one specific user, such
//! as a report being resolved. Announcements, likes, follows, replies and private messages are
//! different kinds of messages and are not stored here.
//!
//! Producers call [send_notification] inside their own business transaction, so the notification
//! is committed or rolled back together with the business change. There is no HTTP endpoint for
//! sending.

use crate::db::notification::{Notification, NotificationDao};
use crate::web::state::AppState;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sqlx::{PgPool, Postgres, Transaction};
use tokio_cron_scheduler::{Job, JobScheduler};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use uuid::Uuid;

/// Notifications older than this are invisible to every API and deleted by the cleanup job.
pub const RETENTION_DAYS: i64 = 180;

const TITLE_MAX_CHARS: usize = 120;
const BODY_MAX_CHARS: usize = 2000;
const INTENT_MAX_BYTES: usize = 1024;

/// Every hour at minute 17.
const CLEANUP_SCHEDULE: &str = "0 17 * * * *";
const CLEANUP_BATCH: i64 = 1000;
const CLEANUP_LOCK_NAME: &str = "notification:cleanup";

/// Where the client goes when the user taps "去查看". Only for navigation; the target page checks
/// existence and permission itself.
///
/// `action` is a dot-separated lowercase name such as `song.comment.view`, and `data` holds the
/// ids the page needs, such as `{ "song_id": 1, "comment_id": 2 }`. Never put URLs or display text
/// in `data`. The client maps each action to a route; add the client mapping together with the
/// producer that first uses an action.
///
/// @since 261005
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentIntent {
    pub action: String,
    pub data: Map<String, Value>,
}

impl ContentIntent {
    pub fn new(action: &str, data: Map<String, Value>) -> Self {
        Self { action: action.to_string(), data }
    }

    /// `None` if the stored value isn't a valid intent, so a bad row still shows its text.
    pub fn from_stored(value: &Value) -> Option<Self> {
        serde_json::from_value::<Self>(value.clone()).ok()
            .filter(|x| is_dotted_name(&x.action))
    }
}

/// A notification to send. Each producer builds this in its own module, including the text.
pub struct NewNotification {
    /// UUIDv7, generated once by the producer and reused when retrying the same send.
    pub id: Uuid,
    pub recipient_uid: i64,
    /// `<domain>.<event>`, e.g. `governance.report_resolved`.
    pub notification_type: &'static str,
    pub title: String,
    pub body: String,
    pub content_intent: Option<ContentIntent>,
    /// When the business event happened. Shown as the notification time.
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    Created,
    /// A notification with the same id and content was already sent.
    AlreadyExists,
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("invalid notification: {0}")]
    Invalid(&'static str),
    /// The id was reused for a different notification, which is a bug in the producer.
    #[error("notification {0} already exists with different content")]
    IdempotencyConflict(Uuid),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Sends a notification inside the caller's transaction. Doesn't commit.
///
/// Sending the same id again with the same content is a no-op. If two transactions insert the
/// same id at once, the later one fails on the primary key; retrying it then gets
/// [SendOutcome::AlreadyExists].
pub async fn send_notification(
    tx: &mut Transaction<'_, Postgres>,
    n: NewNotification,
) -> Result<SendOutcome, SendError> {
    validate(&n)?;
    let content_intent = n.content_intent
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| SendError::Invalid("content_intent is not serializable"))?;

    if let Some(existing) = NotificationDao::get_by_id(&mut **tx, n.id).await? {
        let same = existing.recipient_uid == n.recipient_uid
            && existing.notification_type == n.notification_type
            && existing.title == n.title
            && existing.body == n.body
            && existing.content_intent == content_intent;
        return if same {
            Ok(SendOutcome::AlreadyExists)
        } else {
            Err(SendError::IdempotencyConflict(n.id))
        };
    }

    NotificationDao::insert(&mut **tx, &Notification {
        id: n.id,
        recipient_uid: n.recipient_uid,
        notification_type: n.notification_type.to_string(),
        title: n.title,
        body: n.body,
        content_intent,
        read_time: None,
        create_time: n.occurred_at,
    }).await?;
    Ok(SendOutcome::Created)
}

fn validate(n: &NewNotification) -> Result<(), SendError> {
    if n.id.get_version() != Some(uuid::Version::SortRand) {
        return Err(SendError::Invalid("id must be a UUIDv7"));
    }
    if !is_dotted_name(n.notification_type) {
        return Err(SendError::Invalid("notification_type must look like `domain.event`"));
    }
    if !is_valid_text(&n.title, TITLE_MAX_CHARS, false) {
        return Err(SendError::Invalid("title must be 1..=120 characters without control characters"));
    }
    if !is_valid_text(&n.body, BODY_MAX_CHARS, true) {
        return Err(SendError::Invalid("body must be 1..=2000 characters without control characters other than newlines"));
    }
    if let Some(intent) = &n.content_intent {
        if !is_dotted_name(&intent.action) {
            return Err(SendError::Invalid("content_intent.action must look like `resource.verb`"));
        }
        let size = serde_json::to_vec(intent).map(|x| x.len()).unwrap_or(usize::MAX);
        if size > INTENT_MAX_BYTES {
            return Err(SendError::Invalid("content_intent must be at most 1024 bytes"));
        }
    }
    Ok(())
}

/// At least two segments of `[a-z][a-z0-9_]*` joined by dots.
fn is_dotted_name(s: &str) -> bool {
    let mut segments = 0;
    for segment in s.split('.') {
        let mut chars = segment.chars();
        let valid = chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            return false;
        }
        segments += 1;
    }
    segments >= 2
}

/// Makes user-written text safe for a notification body: normalizes line breaks, turns tabs into
/// spaces and drops other control characters, which [send_notification] would reject.
pub fn to_plain_text(s: &str) -> String {
    s.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', " ")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}

fn is_valid_text(s: &str, max_chars: usize, allow_newline: bool) -> bool {
    let count = s.chars().count();
    (1..=max_chars).contains(&count)
        && s.chars().all(|c| !c.is_control() || (allow_newline && c == '\n'))
}

/// Notifications created before this are expired: hidden from every API and deleted by the
/// cleanup job.
pub fn visible_since() -> DateTime<Utc> {
    Utc::now() - Duration::days(RETENTION_DAYS)
}

/// Newest first. Returns the page and whether there are more.
pub async fn list(
    pool: &PgPool,
    recipient_uid: i64,
    before_id: Option<Uuid>,
    limit: i64,
) -> sqlx::Result<(Vec<Notification>, bool)> {
    let mut items = NotificationDao::list_by_recipient(pool, recipient_uid, before_id, visible_since(), limit + 1).await?;
    let has_more = items.len() as i64 > limit;
    items.truncate(limit as usize);
    Ok((items, has_more))
}

/// `None` if it isn't the recipient's or has expired.
pub async fn get(pool: &PgPool, recipient_uid: i64, id: Uuid) -> sqlx::Result<Option<Notification>> {
    Ok(NotificationDao::get_by_recipient(pool, recipient_uid, id).await?
        .filter(|x| x.create_time >= visible_since()))
}

pub async fn unread_count(pool: &PgPool, recipient_uid: i64) -> sqlx::Result<i64> {
    NotificationDao::count_unread_by_recipient(pool, recipient_uid, visible_since()).await
}

/// Marks it read, keeping the first read time if it already was. Returns the read time and the
/// unread count after marking, or `None` if it isn't the recipient's or has expired.
pub async fn mark_read(pool: &PgPool, recipient_uid: i64, id: Uuid) -> sqlx::Result<Option<(DateTime<Utc>, i64)>> {
    let since = visible_since();
    let mut tx = pool.begin().await?;
    let Some(notification) = NotificationDao::get_by_recipient(&mut *tx, recipient_uid, id).await?
        .filter(|x| x.create_time >= since) else {
        return Ok(None);
    };
    let read_time = match notification.read_time {
        Some(x) => x,
        None => {
            let now = Utc::now();
            if NotificationDao::set_read_time_if_unread(&mut *tx, recipient_uid, id, now).await? {
                now
            } else {
                // Another request marked it read meanwhile; keep its time
                NotificationDao::get_by_recipient(&mut *tx, recipient_uid, id).await?
                    .and_then(|x| x.read_time)
                    .unwrap_or(now)
            }
        }
    };
    let unread_count = NotificationDao::count_unread_by_recipient(&mut *tx, recipient_uid, since).await?;
    tx.commit().await?;
    Ok(Some((read_time, unread_count)))
}

/// Returns how many were marked and the unread count after marking. The count can be non-zero
/// if a notification arrived meanwhile.
pub async fn mark_all_read(pool: &PgPool, recipient_uid: i64) -> sqlx::Result<(u64, i64)> {
    let since = visible_since();
    let mut tx = pool.begin().await?;
    let marked = NotificationDao::set_read_time_all_unread(&mut *tx, recipient_uid, since, Utc::now()).await?;
    let unread_count = NotificationDao::count_unread_by_recipient(&mut *tx, recipient_uid, since).await?;
    tx.commit().await?;
    Ok((marked, unread_count))
}

pub async fn start_cleanup_job(state: AppState, cancel_token: CancellationToken) -> anyhow::Result<()> {
    let mut scheduler = JobScheduler::new().await?;
    scheduler.add(Job::new_async(CLEANUP_SCHEDULE, move |_, _| {
        let state = state.clone();
        Box::pin(async move {
            if let Err(e) = cleanup(&state).await {
                error!("Failed to clean up expired notifications: {e:?}");
            }
        })
    })?).await?;
    scheduler.start().await?;

    tokio::spawn(async move {
        cancel_token.cancelled().await;
        if let Err(e) = scheduler.shutdown().await {
            error!("Failed to shut down notification cleanup scheduler: {e:?}");
        }
    });
    Ok(())
}

/// Deletes expired notifications in batches. Skips if another replica is already doing it.
/// Returns how many were deleted.
pub async fn cleanup(state: &AppState) -> anyhow::Result<u64> {
    let Some(_guard) = state.red_lock.try_lock(CLEANUP_LOCK_NAME).await? else {
        return Ok(0);
    };
    let before = visible_since();
    let mut total = 0;
    loop {
        let deleted = NotificationDao::delete_created_before(&state.sql_pool, before, CLEANUP_BATCH).await?;
        total += deleted;
        if (deleted as i64) < CLEANUP_BATCH {
            break;
        }
        tokio::task::yield_now().await;
    }
    if total > 0 {
        info!("Deleted {total} expired notifications");
    }
    Ok(total)
}
