use crate::db::refresh_token::{IRefreshTokenDao, RefreshToken, RefreshTokenDao};
use crate::db::user::{IUserDao, User, UserDao};
use crate::db::CrudDao;
use crate::service::{mailer, verification_code};
use crate::web::extractors::XRealIP;
use crate::web::jwt::Claims;
use crate::web::result::{WebResult};
use crate::web::state::AppState;
use crate::web::{jwt};
use crate::{err, ok, search, service};
use axum::http::{StatusCode};
use axum::response::{Html};
use axum::routing::get;
use axum::{debug_handler, extract::State, routing::post, Json, Router};
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use axum::extract::Query;
use axum_extra::headers::UserAgent;
use axum_extra::TypedHeader;
use jsonwebtoken::errors::ErrorKind;
use tracing::{error};
use crate::search::user::{UserDocument};
use crate::service::captcha::verify_captcha;
use crate::service::mailer::EmailConfig;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register/email", post(email_register))
        .route("/login/email", post(email_login))
        .route("/send_email_code", post(send_email_code))
        .route("/device/list", get(device_list))
        .route("/device/logout", post(device_logout))
        .route("/refresh_token", post(refresh_token))
        .route("/protected", get(protected))
        .route("/reset_password", post(reset_password))
        .route("/captcha", get(captcha))
        .route("/captcha/generate", get(generate_captcha))
        .route("/captcha/submit", post(submit_captcha))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailRegisterReq {
    pub email: String,
    pub password: String,
    pub code: String,
    pub device_info: String,
    pub captcha_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailRegisterResp {
    pub uid: i64,
    pub generated_username: String,
    pub token: TokenPair,
}

async fn email_register(
    mut state: State<AppState>,
    XRealIP(ip): XRealIP,
    TypedHeader(ua): TypedHeader<UserAgent>,
    req: Json<EmailRegisterReq>,
) -> WebResult<EmailRegisterResp> {
    let captcha = service::captcha::verify_captcha(&mut state.redis_conn, &req.captcha_key).await?;
    if !captcha {
        err!("invalid_captcha", "Invalid captcha")
    }

    // Validate request
    if req.password.len() < 8 {
        err!("invalid_password", "Password must be at least 8 characters")
    }

    if !regex::Regex::new(r"^[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+$")?.is_match(&req.email)
    {
        err!("invalid_email", "Invalid email pattern")
    }

    let pass = verification_code::verify_code(&mut state.redis_conn, &req.email, &req.code).await?;

    if pass {
        // 1. Check user existence
        if UserDao::get_by_email(&state.sql_pool, &req.email).await?.is_some() {
            err!("email_existed", "Email already exists!")
        }

        // 2. Generate username and hash password
        let username = generate_username();
        let password_hash = bcrypt::hash(&req.password, bcrypt::DEFAULT_COST)?;

        // 3. Create user
        let mut entity = User {
            id: 0,
            username: username.clone(),
            email: req.email.clone(),
            password_hash,
            avatar_url: None,
            bio: None,
            gender: None,
            is_banned: false,
            last_login_time: None,
            create_time: Utc::now(),
            update_time: Utc::now(),
        };
        let uid = UserDao::insert(&state.sql_pool, &mut entity).await?;

        search::user::update_user_document(&state.meilisearch, UserDocument {
            id: uid,
            avatar_url: None,
            name: entity.username,
            follower_count: 0,
        }).await?;

        // 4. Generate tokens
        let token =
            generate_token_pairs_and_save(ip, uid, ua.to_string(), req.device_info.clone(), &state.sql_pool)
                .await?;

        ok!(EmailRegisterResp {
            uid: uid,
            generated_username: username,
            token,
        })
    } else {
        err!("invalid_verify_code", "Invalid verify code!")
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginReq {
    pub email: String,
    pub password: String,
    pub device_info: String,
    pub code: Option<String>,
    pub captcha_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginResp {
    pub uid: i64,
    pub username: String,
    pub token: TokenPair,
}

#[async_backtrace::framed]
#[debug_handler]
async fn email_login(
    ip: XRealIP,
    TypedHeader(ua): TypedHeader<UserAgent>,
    mut state: State<AppState>,
    req: Json<LoginReq>,
) -> WebResult<LoginResp> {
    let captcha = service::captcha::verify_captcha(&mut state.redis_conn, &req.captcha_key).await?;
    if !captcha {
        err!("invalid_captcha", "Invalid captcha")
    }

    match &req.code {
        None => {
            // TODO[security](auth): Check if 2fa is required.
            /*let should_2fa = false;
            if should_2fa {
                err!("2fa_required", "2FA is required!")
            }*/

            let user = if let Some(user) = UserDao::get_by_email(&state.sql_pool, &req.email).await? {
                user
            } else {
                err!("password_not_match", "Password not match!")
            };

            if !bcrypt::verify(&req.password, &user.password_hash)? {
                err!("password_not_match", "Password not match!")
            }

            let token = generate_token_pairs_and_save(
                ip.0,
                user.id,
                ua.to_string(),
                req.device_info.clone(),
                &state.sql_pool,
            ).await?;

            let resp = LoginResp {
                uid: user.id,
                username: user.username,
                token,
            };
            ok!(resp)
        }
        Some(_) => {
            // TODO[security](auth): check 2fa code
            err!("invalid_code", "Invalid code")
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshTokenReq {
    pub refresh_token: String,
    pub device_info: String,
}

async fn refresh_token(
    state: State<AppState>,
    XRealIP(ip): XRealIP,
    TypedHeader(ua): TypedHeader<UserAgent>,
    req: Json<RefreshTokenReq>,
) -> WebResult<TokenPair> {
    let claims = jwt::decode_and_validate_refresh_token(&req.refresh_token);
    let claims = match claims {
        Ok(claims) => { claims }
        Err(e) => {
            match e.kind() {
                ErrorKind::InvalidToken => {
                    err!("invalid_token", "Invalid refresh token pattern")
                }
                ErrorKind::InvalidSignature => {
                    err!("invalid_token", "Invalid refresh token signature")
                }
                ErrorKind::ExpiredSignature => {
                    err!("token_expired", "Refresh token expired, please login again")
                }
                _ => { Err(e)? }
            }
        }
    };

    let entry = RefreshTokenDao::get_by_token_id(&state.sql_pool, &claims.jti).await?;

    // Validate
    let entry = if let Some(v) = entry {
        v
    } else {
        err!("token_not_found", "You might using an outdated token")
    };
    if entry.is_revoked {
        err!("token_revoked", "Token revoked")
    }
    if Some(req.device_info.clone()) != entry.device_info {
        err!("inconsistent_device", "Inconsistent device info")
    }

    let uid = entry.user_id;

    let expires_in = Utc::now() + Duration::minutes(5);
    let access_token = jwt::generate_access_token(&uid.to_string(), expires_in.timestamp());

    let token = if entry.expires_time - Utc::now() < Duration::days(7) {
        // When the refresh token is about to expire, generate a new one.
        let (refresh_token, claims) = jwt::generate_refresh_token(&uid.to_string());
        RefreshToken {
            token_id: claims.jti,
            token_value: refresh_token,
            expires_time: DateTime::from_timestamp(claims.exp as i64, 0).unwrap(),
            last_used_time: Some(Utc::now()),
            device_info: Some(req.device_info.clone()),
            ip_address: Some(ip),
            user_agent: Some(ua.to_string()),
            ..entry
        }
    } else {
        // Just use the original token
        RefreshToken {
            last_used_time: Some(Utc::now()),
            device_info: Some(req.device_info.clone()),
            ip_address: Some(ip),
            user_agent: Some(ua.to_string()),
            ..entry
        }
    };

    // Update the token
    RefreshTokenDao::update_by_id(&state.sql_pool, &token).await?;

    ok!(TokenPair {
        access_token,
        refresh_token: token.token_value.clone(),
        expires_in,
    });
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SendVerificationReq {
    pub email: String,
}

#[async_backtrace::framed]
async fn send_email_code(
    State(state): State<AppState>,
    Json(req): Json<SendVerificationReq>,
) -> WebResult<()> {
    let mut redis = state.redis_conn;

    let limit_absent: bool = verification_code::set_limit_nx(&mut redis, &req.email).await?;

    if !limit_absent {
        err!("too_many_requests","Too many requests, please try again later!");
    }

    let code = verification_code::generate_verify_code();

    let email_cfg: EmailConfig = state.config.get_and_parse("email")?;
    mailer::send_verification_code(&email_cfg, &req.email, &code).await?;

    verification_code::set_code(&mut redis, &req.email, &code).await?;
    ok!(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthLoginResp {
    pub first_access: bool,
    pub token: TokenPair,
}

// async fn oauth_github() {
    // https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps
    // scopes = read:user, user:email
    // 1. Build authorize url
    // 2. Github callback
    // 3. Pickup code
    // 4. Read user profile(username, email, avatar)
    // 5. Login/register
    // 6. Return tokens
// }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceListResp {
    pub devices: Vec<DeviceItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceItem {
    pub id: i64,
    pub device_info: Option<String>,
    pub ip_address: Option<String>,
    pub last_used_time: Option<DateTime<Utc>>,
    pub create_time: DateTime<Utc>,
}

async fn device_list(
    State(state): State<AppState>,
    claims: Claims,
) -> WebResult<DeviceListResp> {
    let devices = RefreshTokenDao::list_by_uid(&state.sql_pool, claims.uid()).await?
        .into_iter()
        .map(|x| DeviceItem {
            id: x.id, // Should we use device id instead?
            device_info: x.device_info,
            ip_address: x.ip_address,
            last_used_time: x.last_used_time,
            create_time: x.create_time,
        })
        .collect();
    ok!(DeviceListResp {devices})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceLogoutReq {
    pub device_id: i64,
}

async fn device_logout(
    State(state): State<AppState>,
    claims: Claims,
    req: Json<DeviceLogoutReq>,
) -> WebResult<()> {
    let device = if let Some(x) = RefreshTokenDao::get_by_id(&state.sql_pool, req.device_id).await? {
        x
    } else {
        err!("invalid_device", "Invalid device id");
    };

    if claims.uid() != device.user_id {
        err!("invalid_device", "Invalid device id")
    }

    // TODO[opt](auth): Utilize the `revoked` field?
    RefreshTokenDao::delete_by_id(&state.sql_pool, device.id).await?;
    ok!(())
}

async fn protected(_: Claims) -> WebResult<()> {
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetPasswordReq {
    pub email: String,
    pub code: String,
    pub new_password: String,
    pub logout_all_devices: bool,
    pub captcha_key: String,
}

async fn reset_password(
    mut state: State<AppState>,
    req: Json<ResetPasswordReq>,
) -> WebResult<()> {
    if verification_code::verify_code(&mut state.redis_conn, req.email.as_str(), req.code.as_str()).await? {
        let captcha_pass = verify_captcha(&mut state.redis_conn, req.captcha_key.as_str()).await?;
        if !captcha_pass {
            err!("invalid_captcha", "Invalid captcha")
        }

        let mut tx = state.sql_pool.begin().await?;
        let mut user = if let Some(user) = UserDao::get_by_email(&mut *tx, req.email.as_str()).await? {
            user
        } else {
            err!("invalid_user", "Invalid user")
        };
        user.password_hash = bcrypt::hash(req.new_password.as_str(), bcrypt::DEFAULT_COST)?;
        user.update_time = Utc::now();

        UserDao::update_by_id(&mut *tx, &user).await?;

        if req.logout_all_devices {
            RefreshTokenDao::delete_all_by_uid(&mut *tx, user.id).await?;
        }
        tx.commit().await?;
        ok!(())
    } else {
        err!("invalid_user", "Invalid user")
    }
}

fn generate_username() -> String {
    format!("神人{:08}", rand::rng().random_range(0..100000000))
}

async fn generate_token_pairs_and_save(
    ip: String,
    uid: i64,
    ua: String,
    device_info: String,
    sql_pool: &PgPool,
) -> anyhow::Result<TokenPair> {
    let expires_in = Utc::now() + Duration::minutes(5);
    let access_token = jwt::generate_access_token(&uid.to_string(), expires_in.timestamp());
    let (refresh_token, claims) = jwt::generate_refresh_token(&uid.to_string());

    let entity = RefreshToken {
        id: 0,
        user_id: uid,
        token_id: claims.jti,
        token_value: refresh_token.clone(),
        expires_time: DateTime::from_timestamp(claims.exp as i64, 0).unwrap(),
        create_time: chrono::Utc::now(),
        last_used_time: None,
        device_info: Some(device_info),
        ip_address: Some(ip),
        is_revoked: false,
        user_agent: Some(ua),
    };

    RefreshTokenDao::insert(sql_pool, &entity).await?;

    Ok(TokenPair {
        access_token,
        refresh_token,
        expires_in,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptchaReq {
    pub captcha_key: String,
}

#[debug_handler]
async fn captcha(
    state: State<AppState>,
    _: Query<CaptchaReq>,
) -> (StatusCode, Html<String>) {
    let cfg = match state.config.get_and_parse::<TurnstileCfg>("turnstile") {
        Ok(v) => { v }
        Err(err) => {
            error!("{:?}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, Html("Error".to_string()));
        }
    };

    let html = CAPTCHA_HTML.replace("{{API_BASE_URL}}", cfg.api_base_url.as_str())
        .replace("{{SITE_KEY}}", cfg.site_key.as_str());
    (StatusCode::OK, Html(html))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateCaptchaResp {
    pub captcha_key: String,
    pub url: String,
}

const CAPTCHA_HTML: &str = include_str!("captcha.html");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnstileCfg {
    pub captcha_page_url: String,
    pub api_base_url: String,
    pub site_key: String,
    pub secret_key: String,
}

#[debug_handler]
async fn generate_captcha(
    mut state: State<AppState>,
) -> WebResult<GenerateCaptchaResp> {
    let cfg = state.config.get_and_parse::<TurnstileCfg>("turnstile")?;
    let key = service::captcha::generate_new_captcha(&mut state.redis_conn).await?;

    ok!(GenerateCaptchaResp {
        captcha_key: key.clone(), url: format!("{}?captcha_key={}", cfg.captcha_page_url, key)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitCaptchaReq {
    pub captcha_key: String,
    pub token: String,
}

async fn submit_captcha(
    mut state: State<AppState>,
    req: Json<SubmitCaptchaReq>,
) -> WebResult<()> {
    let cfg = state.config.get_and_parse::<TurnstileCfg>("turnstile")?;
    let pass = service::captcha::submit_captcha(&cfg, &mut state.redis_conn, &req.captcha_key, &req.token).await?;
    if pass {
        ok!(())
    } else {
        err!("captcha_failed", "Captcha verification failed")
    }
}