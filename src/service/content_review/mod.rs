//! Content checked again in the background, because TypeSafe couldn't judge it when it was
//! submitted. Only content that must not pass unchecked comes here, which so far is comments; the
//! content waits unpublished, seen only by its author.
//!
//! The content stays in its own table. Each kind of content that can wait here has an adapter
//! (`comment.rs`) that reads the waiting text and settles it once judged; this module only keeps
//! the queue and retries with backoff, and knows no kind by name. Content that still can't be
//! checked after [GIVE_UP_AFTER] is published, as there is no one to judge it by hand yet; a wrong
//! pass can still be reported.
//!
//! As with [crate::service::ugc], each [ReviewKind] variant carries its adapter and `enum_dispatch`
//! forwards [ReviewAdapter] to it. Adding a kind means a new adapter and a variant for it;
//! [ReviewKind::iter] finds it by itself. The kind's subjects must be content check subjects.
//!
//! Metrics, by subject: `content_review_settled_total` counts reviews as they end, by `outcome`
//! (see [Outcome]), and the gauges `content_review_pending` and
//! `content_review_oldest_pending_seconds` show what still waits, so an alert can fire well before
//! content is published unchecked.

mod comment;

use crate::db::content_review::{ContentReview, ContentReviewDao};
use crate::db::user::UserId;
use crate::service::content_check::{ContentChecker, Verdict};
use crate::util::redlock::RedLock;
use crate::web::state::AppState;
use chrono::{DateTime, Duration, Utc};
use enum_dispatch::enum_dispatch;
use metrics::{counter, gauge};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::HashMap;
use std::time::Instant;
use strum::{EnumIter, IntoEnumIterator};
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

/// Checks for due reviews this often.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// Stops taking new reviews after this long in one run, so a run ends well within the 30s lock.
const RUN_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);
const BATCH_SIZE: i64 = 20;
const LOCK_NAME: &str = "content_review:run";
/// Content that still can't be checked this long after it was submitted is published.
pub const GIVE_UP_AFTER: Duration = Duration::hours(24);

/// A kind of content that can wait here. Each variant carries the kind's adapter, made by
/// [Default] when [EnumIter] lists the kinds.
#[enum_dispatch(ReviewAdapter)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter)]
pub enum ReviewKind {
    Comment(comment::CommentReview),
}

/// What every kind of waiting content provides, implemented once per kind in its own file.
// Only called through the concrete ReviewKind, so the futures' auto traits (Send) are known
#[allow(async_fn_in_trait)]
#[enum_dispatch]
pub trait ReviewAdapter {
    /// The content check subjects this kind's content is checked as, also stored in the queue.
    fn subjects(&self) -> Vec<&'static str>;

    /// Who wrote the content and its text, if it still waits. None if it was deleted or settled
    /// some other way, which ends its review.
    async fn load_pending(&self, pool: &PgPool, id: i64) -> sqlx::Result<Option<(UserId, String)>>;

    /// Publishes the content if `pass`, or rejects it.
    async fn settle(&self, pool: &PgPool, id: i64, pass: bool) -> sqlx::Result<()>;
}

impl ReviewKind {
    pub fn of_subject(subject: &str) -> Option<Self> {
        Self::iter().find(|x| x.subjects().contains(&subject))
    }
}

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
            if let Err(e) = report_pending(&state.sql_pool, Utc::now()).await {
                error!("Failed to report pending content reviews: {e:?}");
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

/// Sets the pending gauges of every subject, 0 for those with nothing waiting. Every replica reports
/// the same numbers, so they are read with `max`.
pub async fn report_pending(pool: &PgPool, now: DateTime<Utc>) -> sqlx::Result<()> {
    let pending: HashMap<_, _> = ContentReviewDao::summarize_pending(pool).await?
        .into_iter()
        .map(|x| (x.subject.clone(), x))
        .collect();
    for subject in ReviewKind::iter().flat_map(|x| x.subjects()) {
        let (count, oldest) = match pending.get(subject) {
            Some(x) => (x.count as f64, (now - x.oldest_create_time).num_seconds().max(0) as f64),
            None => (0.0, 0.0),
        };
        gauge!("content_review_pending", "subject" => subject).set(count);
        gauge!("content_review_oldest_pending_seconds", "subject" => subject).set(oldest);
    }
    Ok(())
}

/// How a review ended, the `outcome` label of `content_review_settled_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
enum Outcome {
    /// Checked and published.
    Pass,
    /// Checked and rejected.
    Block,
    /// Published unchecked after [GIVE_UP_AFTER].
    GaveUp,
    /// Published unchecked because checks are off.
    Unchecked,
    /// Deleted, or settled some other way, before it was checked.
    Gone,
}

fn record_settled(subject: &str, outcome: Outcome) {
    let outcome: &'static str = outcome.into();
    counter!("content_review_settled_total", "subject" => subject.to_string(), "outcome" => outcome).increment(1);
}

async fn review_one(pool: &PgPool, checker: Option<&ContentChecker>, mut review: ContentReview, now: DateTime<Utc>) -> anyhow::Result<()> {
    let Some(kind) = ReviewKind::of_subject(&review.subject) else {
        // Written by a newer version, or by mistake; leave it for an instance that knows it
        warn!(id = review.id, subject = review.subject, "Unknown content review subject");
        review.available_time = now + Duration::hours(1);
        ContentReviewDao::update(pool, &review).await?;
        return Ok(());
    };
    let Some((actor_uid, text)) = kind.load_pending(pool, review.target_id).await? else {
        // Deleted, or settled some other way
        review.done_time = Some(now);
        ContentReviewDao::update(pool, &review).await?;
        record_settled(&review.subject, Outcome::Gone);
        return Ok(());
    };

    review.attempt_count += 1;
    let verdict = match checker {
        Some(checker) => checker.try_check_text(pool, &review.subject, actor_uid, review.target_id, &text).await?,
        // Checks were turned off, as in an emergency; content is published unchecked meanwhile
        None => Some(Verdict::Pass),
    };
    let outcome = match verdict {
        Some(verdict) => {
            kind.settle(pool, review.target_id, verdict == Verdict::Pass).await?;
            review.done_time = Some(now);
            review.last_error = None;
            Some(match (checker, verdict) {
                (None, _) => Outcome::Unchecked,
                (Some(_), Verdict::Pass) => Outcome::Pass,
                (Some(_), _) => Outcome::Block,
            })
        }
        None if now - review.create_time >= GIVE_UP_AFTER => {
            warn!(id = review.id, subject = review.subject, "Publishing content that couldn't be checked for {} attempts", review.attempt_count);
            kind.settle(pool, review.target_id, true).await?;
            review.done_time = Some(now);
            review.last_error = Some("Gave up checking and published".to_string());
            Some(Outcome::GaveUp)
        }
        None => {
            review.available_time = now + retry_delay(review.attempt_count);
            review.last_error = Some("The content check failed; see content_checks".to_string());
            None
        }
    };
    ContentReviewDao::update(pool, &review).await?;
    if let Some(outcome) = outcome {
        record_settled(&review.subject, outcome);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ReviewAdapter, ReviewKind};
    use crate::service::content_check::{SUBJECTS, SUBJECT_SONG_COMMENT};
    use strum::IntoEnumIterator;

    #[test]
    fn every_subject_is_a_check_subject_and_finds_its_kind() {
        for kind in ReviewKind::iter() {
            for subject in kind.subjects() {
                assert!(SUBJECTS.contains(&subject), "{subject}");
                assert_eq!(ReviewKind::of_subject(subject), Some(kind));
            }
        }
        assert!(ReviewKind::of_subject(SUBJECT_SONG_COMMENT).is_some());
    }
}
