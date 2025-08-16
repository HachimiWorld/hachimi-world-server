use std::env;
use crate::db::CrudDao;
use crate::db::refresh_token::{IRefreshTokenDao, RefreshToken, RefreshTokenDao};
use crate::db::user::{IUserDao, User, UserDao};
use crate::web::extractors::XRealIP;
use crate::web::jwt;
use crate::web::jwt::Claims;
use crate::web::result::WebError;
use crate::web::result::{WebResponse, WebResult};
use crate::web::state::AppState;
use crate::{err, ok};
use axum::routing::get;
use axum::{Json, Router, debug_handler, extract::State, routing::post};
use chrono::{DateTime, Duration, Utc};
use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use rand::Rng;
use redis::{AsyncCommands, ExistenceCheck, SetExpiry, SetOptions};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register/email", post(email_register))
        .route("/login/email", post(email_login))
        .route("/send_email_code", post(send_email_code))
        .route("/device/list", get(device_list))
        .route("/device/logout", post(device_logout))
        .route("/refresh_token", post(refresh_token))
        .route("/protected", get(protected))
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailRegisterResp {
    pub uid: String,
    pub generated_username: String,
    pub token: TokenPair,
}

async fn email_register(
    state: State<AppState>,
    XRealIP(ip): XRealIP,
    req: Json<EmailRegisterReq>,
) -> WebResult<EmailRegisterResp> {
    // Validate request
    if req.password.len() < 8 {
        err!("invalid_password", "Password must be at least 8 characters")
    }

    if !regex::Regex::new(r"^[a-zA-Z0-9_.+-]+@[a-zA-Z0-9-]+\.[a-zA-Z0-9-.]+$")?.is_match(&req.email)
    {
        err!("invalid_email", "Invalid email pattern")
    }

    let code: Option<String> = state
        .redis_conn
        .clone()
        .get(format!("email_code:{}", req.email))
        .await?;

    if let Some(code) = code
        && req.code == code
    {
        let _: () = state
            .redis_conn
            .clone()
            .del(format!("email_code:{}", req.email))
            .await?;

        // 1. Check user existence
        // 2. Generate username and hash password
        // 3. Create user
        // 4. Generate tokens
        let user_dao = UserDao::new(state.sql_pool.clone());
        if user_dao.get_by_email(&req.email).await?.is_some() {
            err!("email_existed", "Email already exists!")
        }

        let username = generate_username();
        let password_hash = bcrypt::hash(&req.password, bcrypt::DEFAULT_COST)?;

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
        let uid = user_dao.insert(&mut entity).await?;

        let token =
            generate_token_pairs_and_save(ip, uid, req.device_info.clone(), &state.sql_pool)
                .await?;

        ok!(EmailRegisterResp {
            uid: uid.to_string(),
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginResp {
    pub token: TokenPair,
}

#[async_backtrace::framed]
#[debug_handler]
async fn email_login(
    XRealIP(ip): XRealIP,
    State(state): State<AppState>,
    req: Json<LoginReq>,
) -> WebResult<LoginResp> {
    match &req.code {
        None => {
            // TODO: Check if 2fa is required.
            let user_dao = UserDao::new(state.sql_pool.clone());

            /*let should_2fa = false;
            if should_2fa {
                err!("2fa_required", "2FA is required!")
            }*/

            let user = if let Some(user) = user_dao.get_by_email(&req.email).await? {
                user
            } else {
                err!("password_not_match", "Password not match!")
            };

            if !bcrypt::verify(&req.password, &user.password_hash)? {
                err!("password_not_match", "Password not match!")
            }

            let token = generate_token_pairs_and_save(
                ip,
                user.id,
                req.device_info.clone(),
                &state.sql_pool,
            )
            .await?;

            ok!(LoginResp { token })
        }
        Some(code) => {
            // TODO: check 2fa code
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
    req: Json<RefreshTokenReq>,
) -> WebResult<TokenPair> {
    let token_dao = RefreshTokenDao::new(state.sql_pool.clone());
    let claims = jwt::decode_and_validate_refresh_token(&req.refresh_token)?;

    let entry = token_dao.get_by_token_id(&claims.jti).await?;

    // Validate
    let entry = if let Some(v) = entry {
        v
    } else {
        err!("invalid_token", "Invalid refresh token")
    };
    if entry.is_revoked {
        err!("invalid_token", "Invalid refresh token")
    }
    if Some(req.device_info.clone()) != entry.device_info {
        err!("inconsistent_device", "Inconsistent device info")
    }

    let uid = entry.user_id;

    let expires_in = Utc::now() + Duration::minutes(5);
    let access_token = jwt::generate_access_token(&uid.to_string(), expires_in.timestamp());
    let (refresh_token, claims) = jwt::generate_refresh_token(&uid.to_string());

    let entity = RefreshToken {
        token_id: claims.jti,
        token_value: refresh_token.clone(),
        expires_time: DateTime::from_timestamp(claims.exp as i64, 0).unwrap(),
        create_time: Utc::now(),
        last_used_time: None,
        device_info: Some(req.device_info.clone()),
        ip_address: Some(ip),
        ..entry
    };

    token_dao.update_by_id(&entity).await?;

    ok!(TokenPair {
        access_token,
        refresh_token,
        expires_in,
    });
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SendVerificationReq {
    pub email: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EmailConfig {
    pub host: String,
    pub username: String,
    pub password: String,
    pub no_reply_email: String
}

#[async_backtrace::framed]
async fn send_email_code(
    State(state): State<AppState>,
    Json(req): Json<SendVerificationReq>,
) -> WebResult<()> {
    let mut redis = state.redis_conn.clone();
    let code = generate_verify_code();

    let limit_absent: bool = redis
        .set_options(
            format!("email_code:limited:{}", req.email),
            0,
            SetOptions::default()
                .conditional_set(ExistenceCheck::NX)
                .with_expiration(SetExpiry::EX(60)),
        )
        .await?;

    if !limit_absent {
        err!(
            "too_many_requests",
            "Too many requests, please try again later!"
        );
    }

    // TODO: Extract as service
    let email_cfg: EmailConfig = state.config.get_and_parse("email")?;

    let email_msg = lettre::Message::builder()
        .from(Mailbox::new(
            Some("Hachimi World".to_string()),
            email_cfg.no_reply_email.parse()?,
        ))
        .to(Mailbox::new(None, req.email.parse()?))
        .subject("Your email verification code - Hachimi World")
        .header(ContentType::TEXT_PLAIN)
        .body(String::from(&code))?;
    let creds = Credentials::new(
        email_cfg.username,
        email_cfg.password
    );

    let mailer = SmtpTransport::relay(email_cfg.host.as_str())?
        .credentials(creds)
        .build();
    mailer.send(&email_msg)?;

    let _: () = redis
        .set_ex(format!("email_code:{}", req.email), code, 300)
        .await?;
    ok!(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthLoginResp {
    pub first_access: bool,
    pub token: TokenPair,
}

async fn oauth_github() {
    // https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps
    // scopes = read:user, user:email
    // 1. Build authorize url
    // 2. Github callback
    // 3. Pickup code
    // 4. Read user profile(username, email, avatar)
    // 5. Login/register
    // 6. Return tokens
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceListResp {
    pub devices: Vec<DeviceItem>
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceItem {
    pub id: i64,
    pub device_info: Option<String>,
    pub ip_address: Option<String>,
    pub last_used_time: Option<DateTime<Utc>>,
    pub create_time: DateTime<Utc>
}

async fn device_list(
    State(state): State<AppState>,
    claims: Claims
) -> WebResult<DeviceListResp> {
    let devices = RefreshTokenDao::new(state.sql_pool.clone())
        .list_by_uid(claims.uid()).await?
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
    device_id: i64
}

async fn device_logout(
    State(state): State<AppState>,
    claims: Claims,
    req: Json<DeviceLogoutReq>
) -> WebResult<()> {
    let token_dao = RefreshTokenDao::new(state.sql_pool.clone());
    let device = if let Some(x) = token_dao.get_by_id(req.device_id).await? {
        x
    } else {
        err!("invalid_device", "Invalid device id");
    };

    if claims.uid() != device.user_id {
        err!("invalid_device", "Invalid device id")
    }

    // TODO[opt](auth): Utilize the `revoked` field?
    token_dao.delete_by_id(device.id).await?;
    ok!(())
}

async fn protected(claims: Claims) -> WebResult<()> {
    ok!(())
}

fn generate_verify_code() -> String {
    format!("{:06}", rand::rng().random_range(0..1000000))
}

fn generate_username() -> String {
    format!("神人{:08}", rand::rng().random_range(0..100000000))
}

async fn generate_token_pairs_and_save(
    ip: String,
    uid: i64,
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
    };

    RefreshTokenDao::new(sql_pool.clone())
        .insert(&entity)
        .await?;
    Ok(TokenPair {
        access_token,
        refresh_token,
        expires_in,
    })
}
#[cfg(test)]
mod test {
    use crate::web::routes::auth::generate_verify_code;

    #[test]
    fn test_gen_verify_code() {
        for _ in 0..100 {
            let code = generate_verify_code();
            assert_eq!(6, code.len())
        }
    }
}
