//! Content checked again in the background, because TypeSafe couldn't judge it when it was
//! submitted. Only content that must not pass unchecked comes here, which so far is comments and
//! moments; the content waits here, seen by no one, its author included.
//!
//! One piece of content can need several checks, such as a moment's text and its images: its
//! adapter lists them as [ReviewItem]s, and it's published only once every one passes.
//!
//! The content waits in the review's payload and is saved to its own table once it passes; blocked
//! content is dropped. Each kind of content that can wait here has an adapter (`comment.rs`) that
//! reads its payload and saves it; this module only keeps the queue and retries with backoff, and
//! knows no kind by name. Content that still can't be
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
mod moment;

use crate::db::content_check::ContentCheckDao;
use crate::db::content_review::{ContentReview, ContentReviewDao};
use crate::file_hosting::FileHost;
use crate::db::user::UserId;
use crate::service::content_check::{Checked, ContentChecker, Verdict};
use crate::service::upload;
use crate::util::redlock::RedLock;
use crate::web::state::AppState;
use chrono::{DateTime, Duration, Utc};
use enum_dispatch::enum_dispatch;
use metrics::{counter, gauge};
use serde_json::Value;
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
    /// @since 261106
    Moment(moment::MomentReview),
}

/// One check a piece of waiting content needs.
/// @since 261106
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewItem {
    Text { subject: &'static str, text: String },
    /// An image already stored at `key`, read back from the file host to be checked.
    Image { subject: &'static str, key: String },
}

/// What every kind of waiting content provides, implemented once per kind in its own file.
// Only called through the concrete ReviewKind, so the futures' auto traits (Send) are known
#[allow(async_fn_in_trait)]
#[enum_dispatch]
pub trait ReviewAdapter {
    /// The content check subjects this kind's content is checked as, also stored in the queue.
    fn subjects(&self) -> Vec<&'static str>;

    /// What to check, from the payload the content waits in. Published once every item passes;
    /// dropped if one is blocked.
    fn items(&self, payload: &Value) -> serde_json::Result<Vec<ReviewItem>>;

    /// Saves the content in its payload to its own table, in `tx`, the transaction that ends its
    /// review, once it passed. Returns its new id.
    async fn publish(&self, tx: &mut Transaction<'_, Postgres>, payload: &Value) -> anyhow::Result<i64>;

    /// Called in `tx`, the transaction that ends its review, when an item was blocked and the
    /// content is dropped, such as to tell its author.
    async fn reject(&self, tx: &mut Transaction<'_, Postgres>, payload: &Value) -> anyhow::Result<()>;
}

impl ReviewKind {
    pub fn of_subject(subject: &str) -> Option<Self> {
        Self::iter().find(|x| x.subjects().contains(&subject))
    }
}

/// Adds content to check again, waiting in `payload`, the shape its kind's adapter reads. It gets
/// its id when it's saved, which is then written to the review's `target_id`.
pub async fn enqueue(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    actor_uid: UserId,
    payload: Value,
    now: DateTime<Utc>,
) -> sqlx::Result<()> {
    ContentReviewDao::insert(&mut **tx, &ContentReview {
        id: 0,
        subject: subject.to_string(),
        target_id: 0,
        actor_uid,
        payload,
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
            if let Err(e) = run(&state.sql_pool, &state.red_lock, checker, state.file_host.as_ref(), Utc::now()).await {
                error!("Failed to run content reviews: {e:?}");
            }
            if let Err(e) = report_pending(&state.sql_pool, Utc::now()).await {
                error!("Failed to report pending content reviews: {e:?}");
            }
        }
    });
}

/// Checks reviews due at `now` until none are left or the run budget is used. Skips if another
/// replica is running them. With checks off, waiting content is published. Images to check are read
/// back from `file_host`. Returns how many reviews were tried.
pub async fn run(pool: &PgPool, red_lock: &RedLock, checker: Option<&ContentChecker>, file_host: &dyn FileHost, now: DateTime<Utc>) -> anyhow::Result<usize> {
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
            review_one(pool, checker, file_host, review, now).await?;
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
}

fn record_settled(subject: &str, outcome: Outcome) {
    let outcome: &'static str = outcome.into();
    counter!("content_review_settled_total", "subject" => subject.to_string(), "outcome" => outcome).increment(1);
}

async fn review_one(pool: &PgPool, checker: Option<&ContentChecker>, file_host: &dyn FileHost, mut review: ContentReview, now: DateTime<Utc>) -> anyhow::Result<()> {
    let Some(kind) = ReviewKind::of_subject(&review.subject) else {
        // Written by a newer version, or by mistake; leave it for an instance that knows it
        warn!(id = review.id, subject = review.subject, "Unknown content review subject");
        review.available_time = now + Duration::hours(1);
        ContentReviewDao::update(pool, &review).await?;
        return Ok(());
    };
    let items = match kind.items(&review.payload) {
        Ok(x) => x,
        Err(e) => {
            // Likely written by a version whose payload this one can't read; leave it for one that can
            error!(id = review.id, subject = review.subject, "Unreadable content review payload: {e}");
            review.available_time = now + Duration::hours(1);
            review.last_error = Some(format!("Unreadable payload: {e}"));
            ContentReviewDao::update(pool, &review).await?;
            return Ok(());
        }
    };

    review.attempt_count += 1;
    let (verdict, record_ids) = match checker {
        Some(checker) => check_items(pool, checker, file_host, &review, &items).await?,
        // Checks were turned off, as in an emergency; content is published unchecked meanwhile
        None => (Some(Verdict::Pass), vec![]),
    };
    let outcome = match (checker, verdict) {
        (None, _) => Outcome::Unchecked,
        (Some(_), Some(Verdict::Pass)) => Outcome::Pass,
        (Some(_), Some(Verdict::Blocked(_))) => Outcome::Block,
        (Some(_), None) if now - review.create_time >= GIVE_UP_AFTER => {
            warn!(id = review.id, subject = review.subject, "Publishing content that couldn't be checked for {} attempts", review.attempt_count);
            Outcome::GaveUp
        }
        (Some(_), None) => {
            review.available_time = now + retry_delay(review.attempt_count);
            review.last_error = Some("The content check failed; see content_checks".to_string());
            ContentReviewDao::update(pool, &review).await?;
            return Ok(());
        }
    };

    // Saved and ended together, so a crash can't save it twice
    let mut tx = pool.begin().await?;
    if outcome != Outcome::Block {
        review.target_id = kind.publish(&mut tx, &review.payload).await?;
        ContentCheckDao::set_target_id(&mut *tx, &record_ids, review.target_id).await?;
    } else {
        kind.reject(&mut tx, &review.payload).await?;
    }
    review.done_time = Some(now);
    review.last_error = (outcome == Outcome::GaveUp).then(|| "Gave up checking and published".to_string());
    ContentReviewDao::update(&mut *tx, &review).await?;
    tx.commit().await?;
    record_settled(&review.subject, outcome);
    Ok(())
}

/// Checks every item: blocked if any is, `None` if any can't be checked now, else passed. Stops at
/// the first blocked one, as the content is dropped anyway. Returns the records of the checks too,
/// which name the content once it's saved.
async fn check_items(
    pool: &PgPool,
    checker: &ContentChecker,
    file_host: &dyn FileHost,
    review: &ContentReview,
    items: &[ReviewItem],
) -> anyhow::Result<(Option<Verdict>, Vec<i64>)> {
    let mut unchecked = false;
    let mut record_ids = vec![];
    for item in items {
        let checked = match item {
            ReviewItem::Text { subject, text } => {
                Some(checker.try_check_text(pool, subject, review.actor_uid, 0, text).await?)
            }
            ReviewItem::Image { subject, key } => check_stored_image(pool, checker, file_host, review, subject, key).await?,
        };
        record_ids.extend(checked.as_ref().map(|x| x.record_id));
        match checked.and_then(|x| x.verdict) {
            Some(Verdict::Pass) => {}
            Some(blocked @ Verdict::Blocked(_)) => return Ok((Some(blocked), record_ids)),
            None => unchecked = true,
        }
    }
    Ok((if unchecked { None } else { Some(Verdict::Pass) }, record_ids))
}

/// Checks an image stored at `key`. `None` if it can't be read back, so nothing was checked.
async fn check_stored_image(
    pool: &PgPool,
    checker: &ContentChecker,
    file_host: &dyn FileHost,
    review: &ContentReview,
    subject: &str,
    key: &str,
) -> anyhow::Result<Option<Checked>> {
    let stored = match file_host.download(key).await {
        Ok(x) => x,
        Err(e) => {
            warn!(id = review.id, key, "Failed to read back an image to check: {e:#}");
            return Ok(None);
        }
    };
    let jpeg = tokio::task::spawn_blocking({
        let stored = stored.clone();
        move || upload::to_content_check_jpeg(&stored)
    }).await??;
    // Already stored where it would be served from, which the record keeps either way; a blocked
    // one is never referenced
    Ok(Some(checker.try_check_image(pool, subject, review.actor_uid, 0, &stored, &jpeg, key, key).await?))
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
