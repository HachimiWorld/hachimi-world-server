//! Changing a user's email. There are two ways:
//!
//! - With a code from the old address and one from the new address. This proves the user can
//!   read both, so the change applies at once ([change_with_old_email]).
//! - For users who can't read the old address any more: with the password and a code from the new
//!   address. The change applies after [WAIT] ([request_change_with_wait]). Until then the old
//!   address gets reminders with a link that cancels it; once applied, it gets a notice with a link
//!   that undoes it for [REVERT_WINDOW]. Either link also resets the password and logs out every
//!   device ([revert]), since whoever made the change knew the password.

use crate::config::Config;
use crate::db::email_change_request::{
    EmailChangeRequest, EmailChangeRequestDao, METHOD_OLD_EMAIL, METHOD_WAIT, STATUS_APPLIED,
    STATUS_CANCELLED, STATUS_FAILED, STATUS_PENDING, STATUS_REVERTED,
};
use crate::db::email_outbox::EmailOutboxDao;
use crate::db::refresh_token::{IRefreshTokenDao, RefreshTokenDao};
use crate::db::user::{IUserDao, User, UserDao, UserId};
use crate::db::CrudDao;
use crate::service::email_outbox;
use crate::service::mailer::NotificationEmail;
use crate::web::state::AppState;
use chrono::{DateTime, Duration, Utc};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use tokio_cron_scheduler::{Job, JobScheduler};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// How long a change made without the old address waits before it applies.
pub const WAIT: Duration = Duration::days(7);
/// When the reminders are sent to the old address, counted from the request. The last one is a
/// day before the change applies.
pub const REMINDER_OFFSETS: [Duration; 3] = [Duration::zero(), Duration::days(3), Duration::days(6)];
/// How long the old address can undo a change after it applied.
pub const REVERT_WINDOW: Duration = Duration::days(30);
/// A user can make a change without the old address once in this long.
pub const WAIT_REQUEST_INTERVAL: Duration = Duration::days(30);
/// Wrong passwords allowed per hour when changing without the old address.
const MAX_PASSWORD_FAILURES: i64 = 5;

const APPLY_SCHEDULE: &str = "0 * * * * *";
const APPLY_LOCK_NAME: &str = "email_change:apply";
const APPLY_BATCH: i64 = 100;

/// `account` in the config. Optional; every field has a default.
/// @since 261105
#[derive(Debug, Clone, Deserialize)]
pub struct AccountCfg {
    /// The undo link in emails to the old address: this followed by the token.
    #[serde(default = "default_revert_url")]
    pub email_change_revert_url: String,
}

fn default_revert_url() -> String {
    "https://hachimi.world/app/#/auth/revert_email_change?token=".to_string()
}

impl Default for AccountCfg {
    fn default() -> Self {
        AccountCfg { email_change_revert_url: default_revert_url() }
    }
}

pub fn account_cfg(config: &Config) -> anyhow::Result<AccountCfg> {
    if config.get("account")?.is_some() {
        config.get_and_parse("account")
    } else {
        Ok(AccountCfg::default())
    }
}

pub fn is_valid_email(email: &str) -> bool {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+$").unwrap()
    });
    PATTERN.is_match(email)
}

/// `abcdef@example.com` -> `ab***@example.com`, so an email to the old address doesn't give the
/// whole new address away.
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let kept: String = local.chars().take(2).collect();
            format!("{kept}***@{domain}")
        }
        None => "***".to_string(),
    }
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// A random undo token and its hash.
fn new_revert_token() -> (String, String) {
    let token = hex::encode(rand::random::<[u8; 32]>());
    let hash = hash_token(&token);
    (token, hash)
}

fn format_time(time: DateTime<Utc>) -> String {
    time.with_timezone(&chrono_tz::Asia::Shanghai).format("%Y-%m-%d %H:%M（北京时间）").to_string()
}

fn revert_link(cfg: &AccountCfg, token: &str) -> String {
    format!("{}{}", cfg.email_change_revert_url, token)
}

pub fn reminder_email(
    user_name: &str,
    new_email: &str,
    request_time: DateTime<Utc>,
    effective_time: DateTime<Utc>,
    link: &str,
    nth: usize,
    total: usize,
) -> NotificationEmail {
    NotificationEmail {
        subject: format!("你的基米天堂账号正在更换邮箱（{nth}/{total}）"),
        body: format!(
            "亲爱的 {user_name}：\n\n\
            有人在 {} 申请把你账号的绑定邮箱从本邮箱更换为 {}。因为申请时没有验证本邮箱，这次更换要等到 {} 才会生效。\n\n\
            如果这是你本人的操作，请忽略这封邮件。\n\n\
            如果不是你本人操作，说明你的密码可能已经泄露。请在更换生效前打开下面的链接，取消这次更换并重新设置密码：\n{link}\n\n\
            请不要把这个链接转发给别人。这是第 {nth} 封提醒，共 {total} 封。",
            format_time(request_time), mask_email(new_email), format_time(effective_time)
        ),
    }
}

pub fn applied_with_wait_email(user_name: &str, new_email: &str, revert_until: DateTime<Utc>, link: &str) -> NotificationEmail {
    NotificationEmail {
        subject: "你的基米天堂账号已更换邮箱".to_string(),
        body: format!(
            "亲爱的 {user_name}：\n\n\
            你账号的绑定邮箱已从本邮箱更换为 {}。\n\n\
            如果这是你本人的操作，请忽略这封邮件。\n\n\
            如果不是你本人操作，请在 {} 前打开下面的链接，把邮箱改回本邮箱并重新设置密码：\n{link}\n\n\
            请不要把这个链接转发给别人。",
            mask_email(new_email), format_time(revert_until)
        ),
    }
}

pub fn applied_with_old_email_email(user_name: &str, new_email: &str, time: DateTime<Utc>) -> NotificationEmail {
    NotificationEmail {
        subject: "你的基米天堂账号已更换邮箱".to_string(),
        body: format!(
            "亲爱的 {user_name}：\n\n\
            你账号的绑定邮箱已在 {} 从本邮箱更换为 {}，更换时验证了本邮箱收到的验证码。\n\n\
            如果这是你本人的操作，请忽略这封邮件。如果不是，说明本邮箱和你的账号可能已被他人登录，请尽快修改邮箱密码并联系我们。",
            format_time(time), mask_email(new_email)
        ),
    }
}

/// Counts a wrong password, see [too_many_password_failures].
pub async fn record_password_failure(redis: &mut ConnectionManager, uid: UserId) -> anyhow::Result<()> {
    let key = password_failures_key(uid);
    let count: i64 = redis.incr(&key, 1).await?;
    if count == 1 {
        let _: () = redis.expire(&key, 3600).await?;
    }
    Ok(())
}

/// Whether there were too many wrong passwords in the last hour.
pub async fn too_many_password_failures(redis: &mut ConnectionManager, uid: UserId) -> anyhow::Result<bool> {
    let count: Option<i64> = redis.get(password_failures_key(uid)).await?;
    Ok(count.unwrap_or(0) >= MAX_PASSWORD_FAILURES)
}

fn password_failures_key(uid: UserId) -> String {
    format!("email_change:password_failures:{uid}")
}

/// When the user can next change without the old address, if not now.
pub async fn next_wait_request_time(pool: &PgPool, uid: UserId, now: DateTime<Utc>) -> sqlx::Result<Option<DateTime<Utc>>> {
    let latest = EmailChangeRequestDao::get_latest_effective_by_user_id_since(
        pool, uid, METHOD_WAIT, now - WAIT_REQUEST_INTERVAL,
    ).await?;
    Ok(latest.map(|x| x.create_time + WAIT_REQUEST_INTERVAL))
}

/// Ends the user's pending request, if any, and drops its unsent reminders. Returns whether there was one.
async fn cancel_pending_in_tx(tx: &mut Transaction<'_, Postgres>, uid: UserId, now: DateTime<Utc>) -> sqlx::Result<bool> {
    let Some(pending) = EmailChangeRequestDao::get_pending_by_user_id(&mut **tx, uid).await? else {
        return Ok(false);
    };
    let Some(pending) = EmailChangeRequestDao::get_by_id_for_update(&mut **tx, pending.id).await? else {
        return Ok(false);
    };
    if pending.status != STATUS_PENDING {
        return Ok(false);
    }
    EmailChangeRequestDao::update_status(&mut **tx, pending.id, STATUS_CANCELLED, now).await?;
    EmailOutboxDao::delete_pending_by_ids(&mut **tx, &pending.reminder_email_ids).await?;
    Ok(true)
}

/// Cancels the user's pending request. Returns whether there was one.
pub async fn cancel_pending(pool: &PgPool, uid: UserId) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let cancelled = cancel_pending_in_tx(&mut tx, uid, Utc::now()).await?;
    tx.commit().await?;
    Ok(cancelled)
}

/// Changes the email at once. The caller has checked the codes from both addresses and that
/// `new_email` is free. A pending request of the user is cancelled, as this one replaces it.
pub async fn change_with_old_email(
    pool: &PgPool,
    user: &User,
    new_email: &str,
    ip: Option<String>,
    user_agent: Option<String>,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let mut tx = pool.begin().await?;
    cancel_pending_in_tx(&mut tx, user.id, now).await?;

    let mut updated = user.clone();
    updated.email = new_email.to_string();
    updated.update_time = now;
    UserDao::update_by_id(&mut *tx, &updated).await?;

    EmailChangeRequestDao::insert(&mut *tx, &EmailChangeRequest {
        id: 0,
        user_id: user.id,
        old_email: user.email.clone(),
        new_email: new_email.to_string(),
        method: METHOD_OLD_EMAIL.to_string(),
        status: STATUS_APPLIED.to_string(),
        effective_time: now,
        revert_token_hash: None,
        revert_expire_time: None,
        reminder_email_ids: vec![],
        ip_address: ip,
        user_agent,
        finish_time: Some(now),
        create_time: now,
    }).await?;
    email_outbox::enqueue(&mut tx, &user.email, applied_with_old_email_email(&user.username, new_email, now)).await?;
    tx.commit().await?;
    email_outbox::wake_relay();
    Ok(())
}

/// Starts a change that applies after [WAIT], and queues the reminders to the old address. The
/// caller has checked the password, the code from the new address, that `new_email` is free, that
/// there is no pending request and [next_wait_request_time]. Returns the request.
pub async fn request_change_with_wait(
    pool: &PgPool,
    cfg: &AccountCfg,
    user: &User,
    new_email: &str,
    ip: Option<String>,
    user_agent: Option<String>,
) -> anyhow::Result<EmailChangeRequest> {
    let now = Utc::now();
    let effective_time = now + WAIT;
    let (token, hash) = new_revert_token();
    let link = revert_link(cfg, &token);

    let mut tx = pool.begin().await?;
    let mut reminder_email_ids = Vec::with_capacity(REMINDER_OFFSETS.len());
    for (i, offset) in REMINDER_OFFSETS.iter().enumerate() {
        let email = reminder_email(&user.username, new_email, now, effective_time, &link, i + 1, REMINDER_OFFSETS.len());
        let id = email_outbox::enqueue_at(&mut tx, &user.email, email, now + *offset).await?;
        reminder_email_ids.push(id);
    }

    let mut request = EmailChangeRequest {
        id: 0,
        user_id: user.id,
        old_email: user.email.clone(),
        new_email: new_email.to_string(),
        method: METHOD_WAIT.to_string(),
        status: STATUS_PENDING.to_string(),
        effective_time,
        revert_token_hash: Some(hash),
        // The link in the reminders cancels until the change applies. The notice sent then has a
        // new link that undoes it.
        revert_expire_time: Some(effective_time),
        reminder_email_ids,
        ip_address: ip,
        user_agent,
        finish_time: None,
        create_time: now,
    };
    request.id = EmailChangeRequestDao::insert(&mut *tx, &request).await?;
    tx.commit().await?;
    email_outbox::wake_relay();
    Ok(request)
}

/// Applies the pending requests that are due. Returns how many were looked at.
pub async fn apply_due(pool: &PgPool, cfg: &AccountCfg, now: DateTime<Utc>) -> anyhow::Result<usize> {
    let mut total = 0;
    loop {
        let ids = EmailChangeRequestDao::list_due_pending_ids(pool, now, APPLY_BATCH).await?;
        let len = ids.len();
        for id in ids {
            if let Err(e) = apply_one(pool, cfg, id, now).await {
                error!("Failed to apply email change request {id}: {e:?}");
            }
        }
        total += len;
        if (len as i64) < APPLY_BATCH {
            return Ok(total);
        }
    }
}

async fn apply_one(pool: &PgPool, cfg: &AccountCfg, id: i64, now: DateTime<Utc>) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let Some(request) = EmailChangeRequestDao::get_by_id_for_update(&mut *tx, id).await? else {
        return Ok(());
    };
    if request.status != STATUS_PENDING || request.effective_time > now {
        return Ok(());
    }
    EmailOutboxDao::delete_pending_by_ids(&mut *tx, &request.reminder_email_ids).await?;

    let user = UserDao::get_by_id(&mut *tx, request.user_id).await?;
    let taken = UserDao::get_by_email(&mut *tx, &request.new_email).await?
        .is_some_and(|x| x.id != request.user_id);
    let user = match user {
        Some(user) if user.email == request.old_email && !taken => user,
        _ => {
            warn!("Email change request {id} failed: the user is gone, their email changed, or the new address was taken");
            EmailChangeRequestDao::update_status(&mut *tx, id, STATUS_FAILED, now).await?;
            tx.commit().await?;
            return Ok(());
        }
    };

    let mut updated = user.clone();
    updated.email = request.new_email.clone();
    updated.update_time = now;
    UserDao::update_by_id(&mut *tx, &updated).await?;

    let (token, hash) = new_revert_token();
    let revert_until = now + REVERT_WINDOW;
    sqlx::query!(
        "UPDATE email_change_requests
        SET status = $2, finish_time = $3, revert_token_hash = $4, revert_expire_time = $5
        WHERE id = $1",
        id, STATUS_APPLIED, now, hash, revert_until
    ).execute(&mut *tx).await?;
    email_outbox::enqueue(
        &mut tx,
        &request.old_email,
        applied_with_wait_email(&user.username, &request.new_email, revert_until, &revert_link(cfg, &token)),
    ).await?;
    tx.commit().await?;
    email_outbox::wake_relay();
    info!("Applied email change request {id} of user {}", request.user_id);
    Ok(())
}

pub async fn start_apply_job(state: AppState, cancel_token: CancellationToken) -> anyhow::Result<()> {
    let mut scheduler = JobScheduler::new().await?;
    scheduler.add(Job::new_async(APPLY_SCHEDULE, move |_, _| {
        let state = state.clone();
        Box::pin(async move {
            let result = async {
                let Some(_guard) = state.red_lock.try_lock(APPLY_LOCK_NAME).await? else {
                    return Ok(0);
                };
                let cfg = account_cfg(&state.config)?;
                apply_due(&state.sql_pool, &cfg, Utc::now()).await
            }.await;
            if let Err(e) = result {
                error!("Failed to apply email changes: {e:?}");
            }
        })
    })?).await?;
    scheduler.start().await?;

    tokio::spawn(async move {
        cancel_token.cancelled().await;
        if let Err(e) = scheduler.shutdown().await {
            error!("Failed to shut down email change scheduler: {e:?}");
        }
    });
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum RevertError {
    #[error("The link is invalid, used or expired")]
    InvalidToken,
    #[error("The old email is used by another account now")]
    OldEmailTaken,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<sqlx::Error> for RevertError {
    fn from(value: sqlx::Error) -> Self {
        RevertError::Other(value.into())
    }
}

/// Follows the link sent to the old address: cancels the change if it's pending, or puts the old
/// address back if it applied, even if the email changed again since. Either way sets the new
/// password and logs out every device.
pub async fn revert(pool: &PgPool, token: &str, new_password_hash: &str, now: DateTime<Utc>) -> Result<(), RevertError> {
    let mut tx = pool.begin().await?;
    let request = EmailChangeRequestDao::get_by_revert_token_hash_for_update(&mut *tx, &hash_token(token)).await?
        .ok_or(RevertError::InvalidToken)?;
    let usable = matches!(request.status.as_str(), STATUS_PENDING | STATUS_APPLIED)
        && request.revert_expire_time.is_some_and(|x| now < x);
    if !usable {
        return Err(RevertError::InvalidToken);
    }
    let mut user = UserDao::get_by_id(&mut *tx, request.user_id).await?
        .ok_or(RevertError::InvalidToken)?;

    if request.status == STATUS_PENDING {
        EmailChangeRequestDao::update_status(&mut *tx, request.id, STATUS_CANCELLED, now).await?;
        EmailOutboxDao::delete_pending_by_ids(&mut *tx, &request.reminder_email_ids).await?;
    } else {
        if let Some(other) = UserDao::get_by_email(&mut *tx, &request.old_email).await?
            && other.id != user.id
        {
            return Err(RevertError::OldEmailTaken);
        }
        // A change made since, e.g. at once from the address this one switched to, goes too
        cancel_pending_in_tx(&mut tx, user.id, now).await?;
        EmailChangeRequestDao::update_status(&mut *tx, request.id, STATUS_REVERTED, now).await?;
        user.email = request.old_email.clone();
    }

    user.password_hash = new_password_hash.to_string();
    user.update_time = now;
    UserDao::update_by_id(&mut *tx, &user).await?;
    RefreshTokenDao::delete_all_by_uid(&mut *tx, user.id).await?;
    tx.commit().await?;
    info!("Email change request {} of user {} was undone from the old address", request.id, user.id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_email() {
        assert_eq!(mask_email("abcdef@example.com"), "ab***@example.com");
        assert_eq!(mask_email("a@example.com"), "a***@example.com");
        assert_eq!(mask_email("invalid"), "***");
    }

    #[test]
    fn test_is_valid_email() {
        assert!(is_valid_email("a.b+c@example.com"));
        assert!(!is_valid_email("a@b"));
        assert!(!is_valid_email("change_email:1:a@b.com"));
    }

    #[test]
    fn test_revert_token() {
        let (token, hash) = new_revert_token();
        assert_eq!(token.len(), 64);
        assert_eq!(hash, hash_token(&token));
        assert_ne!(token, new_revert_token().0);
    }
}
