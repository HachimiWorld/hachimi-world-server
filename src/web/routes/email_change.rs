//! `/auth/email`: changing the account email. See [crate::service::email_change].

use crate::db::email_change_request::{EmailChangeRequest, EmailChangeRequestDao};
use crate::db::user::{IUserDao, User, UserDao};
use crate::db::CrudDao;
use crate::service::email_change::{self, RevertError};
use crate::service::mailer::{self, EmailConfig};
use crate::service::verification_code::{self, CodeKeys};
use crate::web::extractors::XRealIP;
use crate::web::jwt::Claims;
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::state::AppState;
use crate::{common, err, ok};
use aide::axum::routing::{get, post};
use aide::axum::ApiRouter;
use axum::extract::State;
use axum::Json;
use crate::web::extractors::ClientUserAgent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub fn router() -> ApiRouter<AppState> {
    ApiRouter::new()
        .api_route("/info", get(info))
        .api_route("/send_old_code", post(send_old_code))
        .api_route("/send_new_code", post(send_new_code))
        .api_route("/change", post(change))
        .api_route("/request_change", post(request_change))
        .api_route("/cancel_change", post(cancel_change))
        .api_route("/revert_change", post(revert_change))
}

/// A change waiting to apply.
/// @since 261105
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PendingEmailChange {
    pub new_email: String,
    pub effective_time: DateTime<Utc>,
    pub create_time: DateTime<Utc>,
}

impl From<EmailChangeRequest> for PendingEmailChange {
    fn from(value: EmailChangeRequest) -> Self {
        PendingEmailChange {
            new_email: value.new_email,
            effective_time: value.effective_time,
            create_time: value.create_time,
        }
    }
}

/// @since 261105
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EmailInfoResp {
    pub email: String,
    pub pending: Option<PendingEmailChange>,
    /// When a change without the old email can be requested again, if not now.
    pub next_request_time: Option<DateTime<Utc>>,
}

async fn current_user(state: &AppState, claims: &Claims) -> anyhow::Result<User> {
    UserDao::get_by_id(&state.sql_pool, claims.uid()).await?
        .ok_or_else(|| anyhow::anyhow!("User {} not found", claims.uid()))
}

async fn info(State(state): State<AppState>, claims: Claims) -> WebResult<EmailInfoResp> {
    let user = current_user(&state, &claims).await?;
    let pending = EmailChangeRequestDao::get_pending_by_user_id(&state.sql_pool, user.id).await?;
    let next_request_time = email_change::next_wait_request_time(&state.sql_pool, user.id, Utc::now()).await?;
    ok!(EmailInfoResp {
        email: user.email,
        pending: pending.map(Into::into),
        next_request_time,
    })
}

/// Sends a code to `to`, stored under `keys`.
async fn send_code(state: &AppState, keys: &CodeKeys, to: &str) -> WebResult<()> {
    let mut redis = state.redis_conn.clone();
    if !verification_code::set_limit_nx_with(&mut redis, keys).await? {
        err!("too_many_requests", "Too many requests, please try again later!");
    }
    let code = verification_code::generate_verify_code();
    let email_cfg: EmailConfig = state.config.get_and_parse("email")?;
    mailer::send_verification_code(&email_cfg, to, &code).await?;
    verification_code::set_code_with(&mut redis, keys, &code).await?;
    ok!(())
}

/// Sends a code to the current email.
async fn send_old_code(State(state): State<AppState>, claims: Claims) -> WebResult<()> {
    let user = current_user(&state, &claims).await?;
    send_code(&state, &CodeKeys::change_email_old(user.id), &user.email).await
}

/// @since 261105
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SendNewEmailCodeReq {
    pub new_email: String,
}

/// Checks `new_email` can be changed to. Returns it trimmed.
async fn check_new_email(state: &AppState, user: &User, new_email: &str) -> Result<String, WebError<CommonError>> {
    let new_email = new_email.trim().to_string();
    if !email_change::is_valid_email(&new_email) {
        return Err(common!("invalid_email", "Invalid email pattern"));
    }
    if new_email == user.email {
        return Err(common!("same_email", "This is your current email"));
    }
    if UserDao::get_by_email(&state.sql_pool, &new_email).await?.is_some() {
        return Err(common!("email_existed", "Email already exists!"));
    }
    Ok(new_email)
}

/// Sends a code to the email to change to.
async fn send_new_code(
    State(state): State<AppState>,
    claims: Claims,
    req: Json<SendNewEmailCodeReq>,
) -> WebResult<()> {
    let user = current_user(&state, &claims).await?;
    let new_email = check_new_email(&state, &user, &req.new_email).await?;
    send_code(&state, &CodeKeys::change_email_new(user.id, &new_email), &new_email).await
}

/// @since 261105
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ChangeEmailReq {
    pub new_email: String,
    /// From `/send_old_code`
    pub old_code: String,
    /// From `/send_new_code`
    pub new_code: String,
}

/// Changes the email at once, with codes from both emails.
async fn change(
    State(state): State<AppState>,
    claims: Claims,
    XRealIP(ip): XRealIP,
    ClientUserAgent(ua): ClientUserAgent,
    req: Json<ChangeEmailReq>,
) -> WebResult<()> {
    let user = current_user(&state, &claims).await?;
    let new_email = check_new_email(&state, &user, &req.new_email).await?;

    let mut redis = state.redis_conn.clone();
    if !verification_code::verify_code_with(&mut redis, &CodeKeys::change_email_old(user.id), &req.old_code).await? {
        err!("invalid_old_code", "Invalid code from the current email")
    }
    if !verification_code::verify_code_with(&mut redis, &CodeKeys::change_email_new(user.id, &new_email), &req.new_code).await? {
        err!("invalid_new_code", "Invalid code from the new email")
    }

    email_change::change_with_old_email(&state.sql_pool, &user, &new_email, Some(ip), Some(ua.to_string())).await?;
    ok!(())
}

/// @since 261105
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RequestEmailChangeReq {
    pub new_email: String,
    /// From `/send_new_code`
    pub new_code: String,
    pub password: String,
}

/// For users who can't read their current email: changes it after a wait, with the password and a
/// code from the new email.
async fn request_change(
    State(state): State<AppState>,
    claims: Claims,
    XRealIP(ip): XRealIP,
    ClientUserAgent(ua): ClientUserAgent,
    req: Json<RequestEmailChangeReq>,
) -> WebResult<PendingEmailChange> {
    let user = current_user(&state, &claims).await?;
    let new_email = check_new_email(&state, &user, &req.new_email).await?;

    if EmailChangeRequestDao::get_pending_by_user_id(&state.sql_pool, user.id).await?.is_some() {
        err!("pending_exists", "There is already a pending email change")
    }
    if let Some(next) = email_change::next_wait_request_time(&state.sql_pool, user.id, Utc::now()).await? {
        err!("too_frequent", "You can request again after {}", next.to_rfc3339())
    }

    let mut redis = state.redis_conn.clone();
    if email_change::too_many_password_failures(&mut redis, user.id).await? {
        err!("too_many_attempts", "Too many wrong passwords, please try again later")
    }
    if !bcrypt::verify(&req.password, &user.password_hash)? {
        email_change::record_password_failure(&mut redis, user.id).await?;
        err!("password_not_match", "Password not match!")
    }
    if !verification_code::verify_code_with(&mut redis, &CodeKeys::change_email_new(user.id, &new_email), &req.new_code).await? {
        err!("invalid_new_code", "Invalid code from the new email")
    }

    let cfg = email_change::account_cfg(&state.config)?;
    let request = email_change::request_change_with_wait(
        &state.sql_pool, &cfg, &user, &new_email, Some(ip), Some(ua.to_string()),
    ).await?;
    ok!(request.into())
}

/// Cancels the pending change, if any.
async fn cancel_change(State(state): State<AppState>, claims: Claims) -> WebResult<()> {
    email_change::cancel_pending(&state.sql_pool, claims.uid()).await?;
    ok!(())
}

/// @since 261105
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RevertEmailChangeReq {
    /// From the link in the email to the old address
    pub token: String,
    pub new_password: String,
}

/// From the link sent to the old email: cancels or undoes the change, sets a new password and logs
/// out every device. No login needed.
async fn revert_change(State(state): State<AppState>, req: Json<RevertEmailChangeReq>) -> WebResult<()> {
    if req.new_password.len() < 8 {
        err!("invalid_password", "Password must be at least 8 characters")
    }
    let password_hash = bcrypt::hash(&req.new_password, bcrypt::DEFAULT_COST)?;
    match email_change::revert(&state.sql_pool, req.token.trim(), &password_hash, Utc::now()).await {
        Ok(()) => ok!(()),
        Err(RevertError::InvalidToken) => err!("invalid_token", "The link is invalid, used or expired"),
        Err(RevertError::OldEmailTaken) => err!("old_email_taken", "The old email is used by another account now"),
        Err(RevertError::Other(e)) => Err(e.into()),
    }
}
