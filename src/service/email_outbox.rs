//! Transactional outbox for notification emails. Producers call [enqueue] inside the transaction of
//! the change the email describes, and the relay sends committed emails in the background,
//! retrying failures with backoff. Delivery is at least once: if the server stops right after SMTP
//! accepts an email but before it is marked sent, it is sent again.
//!
//! Verification code emails don't go through here, as the user is waiting for them.

use crate::db::email_outbox::{EmailOutbox, EmailOutboxDao};
use crate::service::mailer::{self, EmailConfig, NotificationEmail};
use crate::util::redlock::RedLock;
use crate::web::state::AppState;
use chrono::{Duration, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Instant;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

/// Checks for due emails this often, besides being woken by [wake_relay].
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
/// Stops taking new emails after this long in one run, so a run ends well within the 30s lock.
const RUN_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);
const BATCH_SIZE: i64 = 20;
const RELAY_LOCK_NAME: &str = "email_outbox:relay";
/// Gives up after this many failed attempts.
const MAX_ATTEMPTS: i32 = 8;

const CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);
const CLEANUP_BATCH: i64 = 1000;
const SENT_RETENTION_DAYS: i64 = 30;
const DEAD_RETENTION_DAYS: i64 = 90;

static RELAY_WAKE: Notify = Notify::const_new();

/// Adds an email to the outbox. It is sent after `tx` commits, and never if it rolls back. Call
/// [wake_relay] after committing to send it right away.
pub async fn enqueue(tx: &mut Transaction<'_, Postgres>, to: &str, email: NotificationEmail) -> sqlx::Result<Uuid> {
    let now = Utc::now();
    let id = Uuid::now_v7();
    EmailOutboxDao::insert(&mut **tx, &EmailOutbox {
        id,
        to_address: to.to_string(),
        subject: email.subject,
        body: email.body,
        available_time: now,
        attempt_count: 0,
        sent_time: None,
        dead_time: None,
        last_error: None,
        create_time: now,
    }).await?;
    Ok(id)
}

/// Lets the relay send newly committed emails now instead of at the next poll.
pub fn wake_relay() {
    RELAY_WAKE.notify_one();
}

/// How long to wait after the `attempt`th failed attempt.
fn retry_delay(attempt: i32) -> Duration {
    match attempt {
        1 => Duration::minutes(1),
        2 => Duration::minutes(5),
        3 => Duration::minutes(30),
        _ => Duration::hours(2),
    }
}

pub fn start_relay(state: AppState, cancel_token: CancellationToken) {
    tokio::spawn(async move {
        let mut last_cleanup: Option<Instant> = None;
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => break,
                _ = RELAY_WAKE.notified() => {}
                _ = tokio::time::sleep(POLL_INTERVAL) => {}
            }
            let email_cfg = match state.config.get_and_parse::<EmailConfig>("email") {
                Ok(x) => x,
                Err(e) => {
                    error!("Failed to read email config, emails are not sent: {e:?}");
                    continue;
                }
            };
            if let Err(e) = relay(&state.sql_pool, &state.red_lock, &email_cfg).await {
                error!("Failed to relay emails: {e:?}");
            }
            if last_cleanup.is_none_or(|x| x.elapsed() >= CLEANUP_INTERVAL) {
                last_cleanup = Some(Instant::now());
                if let Err(e) = cleanup(&state.sql_pool).await {
                    error!("Failed to clean up the email outbox: {e:?}");
                }
            }
        }
    });
}

/// Sends due emails until none are left or the run budget is used. Skips if another replica is
/// relaying. Returns how many emails were attempted.
pub async fn relay(pool: &PgPool, red_lock: &RedLock, email_cfg: &EmailConfig) -> anyhow::Result<usize> {
    // Checked first so idle polls don't take the lock
    if !EmailOutboxDao::exists_pending_available_at(pool, Utc::now()).await? {
        return Ok(0);
    }
    let Some(_guard) = red_lock.try_lock(RELAY_LOCK_NAME).await? else {
        return Ok(0);
    };

    let started = Instant::now();
    let mut attempted = 0;
    loop {
        let due = EmailOutboxDao::list_pending_available_at(pool, Utc::now(), BATCH_SIZE).await?;
        let batch_len = due.len();
        for email in due {
            if started.elapsed() >= RUN_BUDGET {
                return Ok(attempted);
            }
            let result = send(email_cfg, &email).await;
            record_attempt(pool, email, result).await?;
            attempted += 1;
        }
        if (batch_len as i64) < BATCH_SIZE {
            return Ok(attempted);
        }
    }
}

async fn send(email_cfg: &EmailConfig, email: &EmailOutbox) -> anyhow::Result<()> {
    let email_cfg = email_cfg.clone();
    let to = email.to_address.clone();
    let content = NotificationEmail { subject: email.subject.clone(), body: email.body.clone() };
    tokio::task::spawn_blocking(move || mailer::send_notification_blocking(&email_cfg, &to, &content)).await?
}

async fn record_attempt(pool: &PgPool, mut email: EmailOutbox, result: anyhow::Result<()>) -> sqlx::Result<()> {
    let now = Utc::now();
    email.attempt_count += 1;
    match result {
        Ok(()) => {
            email.sent_time = Some(now);
            email.last_error = None;
        }
        Err(e) => {
            let error = format!("{e:#}");
            if email.attempt_count >= MAX_ATTEMPTS {
                error!(id = %email.id, "Gave up sending an email after {} attempts: {error}", email.attempt_count);
                email.dead_time = Some(now);
            } else {
                warn!(id = %email.id, "Failed to send an email, attempt {}: {error}", email.attempt_count);
                email.available_time = now + retry_delay(email.attempt_count);
            }
            email.last_error = Some(error);
        }
    }
    EmailOutboxDao::update_delivery(pool, &email).await
}

/// Deletes emails sent more than 30 days ago and ones given up more than 90 days ago, as they hold
/// email addresses. Returns how many were deleted.
pub async fn cleanup(pool: &PgPool) -> sqlx::Result<u64> {
    let now = Utc::now();
    let mut total = 0;
    loop {
        let deleted = EmailOutboxDao::delete_sent_before(pool, now - Duration::days(SENT_RETENTION_DAYS), CLEANUP_BATCH).await?;
        total += deleted;
        if (deleted as i64) < CLEANUP_BATCH { break; }
    }
    loop {
        let deleted = EmailOutboxDao::delete_dead_before(pool, now - Duration::days(DEAD_RETENTION_DAYS), CLEANUP_BATCH).await?;
        total += deleted;
        if (deleted as i64) < CLEANUP_BATCH { break; }
    }
    if total > 0 {
        info!("Deleted {total} old emails from the outbox");
    }
    Ok(total)
}
