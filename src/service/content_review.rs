//! Content checked again in the background, because TypeSafe couldn't judge it when it was
//! submitted. Only content that must not pass unchecked comes here, which so far is comments; the
//! content waits unpublished, seen only by its author.
//!
//! The content stays in its own table, and its module says how to read it and what to do with the
//! verdict; this module only keeps the queue and retries with backoff. Content that still can't be
//! checked after [GIVE_UP_AFTER] is published, as there is no one to judge it by hand yet; a wrong
//! pass can still be reported.

use crate::db::comment::CommentId;
use crate::db::content_review::{ContentReview, ContentReviewDao};
use crate::service::comment::{self, CommentTarget};
use crate::service::content_check::{ContentChecker, Verdict};
use crate::util::redlock::RedLock;
use crate::web::state::AppState;
use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Checks for due reviews this often.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// Stops taking new reviews after this long in one run, so a run ends well within the 30s lock.
const RUN_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);
const BATCH_SIZE: i64 = 20;
const LOCK_NAME: &str = "content_review:run";
/// Content that still can't be checked this long after it was submitted is published.
pub const GIVE_UP_AFTER: Duration = Duration::hours(24);

/// Adds content to check again in `tx`, the transaction that saves it as waiting.
pub async fn enqueue(tx: &mut Transaction<'_, Postgres>, subject: &str, target_id: i64, now: DateTime<Utc>) -> sqlx::Result<()> {
    ContentReviewDao::insert(&mut **tx, &ContentReview {
        id: 0,
        subject: subject.to_string(),
        target_id,
        attempt_count: 0,
        available_time: now + retry_delay(0),
        done_time: None,
        last_error: None,
        create_time: now,
    }).await?;
    Ok(())
}

/// How long to wait after the `attempt`th failed attempt; the 0th is the check when it was submitted.
fn retry_delay(attempt: i32) -> Duration {
    match attempt {
        0 => Duration::minutes(1),
        1 => Duration::minutes(5),
        2 => Duration::minutes(15),
        3 => Duration::minutes(30),
        _ => Duration::hours(1),
    }
}

pub fn start_worker(state: AppState, cancel_token: CancellationToken) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => break,
                _ = tokio::time::sleep(POLL_INTERVAL) => {}
            }
            let checker = state.content_checker.as_deref();
            if let Err(e) = run(&state.sql_pool, &state.red_lock, checker, Utc::now()).await {
                error!("Failed to run content reviews: {e:?}");
            }
        }
    });
}

/// Checks reviews due at `now` until none are left or the run budget is used. Skips if another
/// replica is running them. With checks off, waiting content is published. Returns how many reviews
/// were tried.
pub async fn run(pool: &PgPool, red_lock: &RedLock, checker: Option<&ContentChecker>, now: DateTime<Utc>) -> anyhow::Result<usize> {
    // Checked first so idle polls don't take the lock
    if !ContentReviewDao::exists_pending_available_at(pool, now).await? {
        return Ok(0);
    }
    let Some(_guard) = red_lock.try_lock(LOCK_NAME).await? else {
        return Ok(0);
    };

    let started = Instant::now();
    let mut tried = 0;
    loop {
        let due = ContentReviewDao::list_pending_available_at(pool, now, BATCH_SIZE).await?;
        let batch_len = due.len();
        for review in due {
            if started.elapsed() >= RUN_BUDGET {
                return Ok(tried);
            }
            review_one(pool, checker, review, now).await?;
            tried += 1;
        }
        if (batch_len as i64) < BATCH_SIZE {
            return Ok(tried);
        }
    }
}

async fn review_one(pool: &PgPool, checker: Option<&ContentChecker>, mut review: ContentReview, now: DateTime<Utc>) -> anyhow::Result<()> {
    let Some(target) = CommentTarget::of_subject(&review.subject) else {
        // Written by a newer version, or by mistake; leave it for an instance that knows it
        warn!(id = review.id, subject = review.subject, "Unknown content review subject");
        review.available_time = now + Duration::hours(1);
        ContentReviewDao::update(pool, &review).await?;
        return Ok(());
    };
    let comment_id = CommentId(review.target_id);
    let Some((actor_uid, text)) = comment::load_pending(pool, comment_id).await? else {
        // Deleted, or settled some other way
        review.done_time = Some(now);
        ContentReviewDao::update(pool, &review).await?;
        return Ok(());
    };

    review.attempt_count += 1;
    let verdict = match checker {
        Some(checker) => checker.try_check_text(pool, target.subject(), actor_uid, review.target_id, &text).await?,
        // Checks were turned off, as in an emergency; content is published unchecked meanwhile
        None => Some(Verdict::Pass),
    };
    match verdict {
        Some(verdict) => {
            comment::settle_pending(pool, comment_id, verdict == Verdict::Pass).await?;
            review.done_time = Some(now);
            review.last_error = None;
        }
        None if now - review.create_time >= GIVE_UP_AFTER => {
            info!(id = review.id, subject = review.subject, "Publishing content that couldn't be checked for {} attempts", review.attempt_count);
            comment::settle_pending(pool, comment_id, true).await?;
            review.done_time = Some(now);
            review.last_error = Some("Gave up checking and published".to_string());
        }
        None => {
            review.available_time = now + retry_delay(review.attempt_count);
            review.last_error = Some("The content check failed; see content_checks".to_string());
        }
    }
    ContentReviewDao::update(pool, &review).await?;
    Ok(())
}
