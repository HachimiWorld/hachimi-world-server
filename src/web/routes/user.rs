use std::io::Cursor;
use anyhow::Context;
use async_backtrace::framed;
use crate::db::CrudDao;
use crate::db::user::{IUserDao, UserDao};
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::result::{WebError, WebResponse};
use crate::web::state::AppState;
use crate::{err, ok};
use axum::routing::post;
use axum::{Json, Router, extract::State, routing::get};
use axum::extract::Multipart;
use chrono::Utc;
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/greet", get(greet))
        .route("/profile", post(get_profile))
        .route("/update_profile", post(update_profile))
        .route("/set_avatar", post(set_avatar))
}

async fn greet() -> WebResult<&'static str> {
    ok!("Hello from Hachimi World!")
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct GetProfileReq {
    pub uid: String,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PublicUserProfile {
    pub uid: String,
    pub username: String,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub gender: Option<i32>,
    pub is_banned: bool,
}

async fn get_profile(
    state: State<AppState>,
    req: Json<GetProfileReq>,
) -> WebResult<PublicUserProfile> {
    // Fetch user from db
    let user_dao = UserDao::new(state.sql_pool.clone());
    let user = if let Some(x) = user_dao.get_by_id(req.uid.parse()?).await? {
        x
    } else {
        err!("not_found", "User not found")
    };

    let mapped = PublicUserProfile {
        uid: user.id.to_string(),
        username: user.username,
        avatar_url: user.avatar_url,
        bio: user.bio,
        gender: user.gender,
        is_banned: user.is_banned,
    };

    ok!(mapped)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProfileReq {
    pub username: String,
    pub bio: Option<String>,
    pub gender: Option<i32>,
}

async fn update_profile(
    claims: Claims,
    State(state): State<AppState>,
    req: Json<UpdateProfileReq>,
) -> WebResult<()> {
    let user_dao = UserDao::new(state.sql_pool.clone());

    // Validate input
    if req.username.is_empty() {
        err!("invalid_username", "Username cannot be empty");
    }
    if req.username.chars().count() > 10 {
        err!(
            "invalid_username",
            "Username must be less than 10 characters"
        );
    }
    if user_dao.get_by_username(&req.username).await?.is_some() {
        err!("username_exists", "Username already exists");
    }

    if let Some(ref bio) = req.bio {
        if bio.chars().count() > 300 {
            err!("invalid_vio", "Bio must be 300 characters or less");
        }
    }

    if let Some(gender) = req.gender
        && gender != 0
        && gender != 1
    {
        err!("invalid_gender", "Gender must be 'null', 0, or 1");
    }

    // Update user profile
    let mut user = if let Some(x) = user_dao.get_by_id(claims.uid()).await? {
        x
    } else {
        err!("not_found", "User not found")
    };
    user.username = req.username.clone();
    user.gender = req.gender;
    user.bio = req.bio.clone();
    user.update_time = Utc::now();
    user_dao.update_by_id(&user).await?;

    ok!(())
}

#[framed]
async fn set_avatar(
    claims: Claims,
    state: State<AppState>,
    mut multipart: Multipart,
) -> WebResult<()> {
    // TODO[opt]: Limit access rate
    let user_dao = UserDao::new(state.sql_pool.clone());
    let mut user = if let Some(x) = user_dao.get_by_id(claims.uid()).await? {
        x
    } else {
        err!("not_found", "User not found")
    };

    let data_field = multipart
        .next_field()
        .await?
        .with_context(|| "No data field found")?;
    let bytes = data_field.bytes().await?;

    let start = std::time::Instant::now();

    // Validate image
    if bytes.len() > 8 * 1024 * 1024 {
        err!("image_too_large", "Image size must be less than 8MB");
    }
    let image = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| WebError::common("invalid_image", "Invalid image"))?
        .decode()
        .map_err(|_| WebError::common("invalid_image", "Invalid image"))?;

    // Resize image
    // TODO[opt](user): Change filter type to Lanczos3 for better image quality
    let resized = image.resize_to_fill(128, 128, FilterType::Nearest);
    let mut output = Cursor::new(Vec::<u8>::new());
    resized.write_to(&mut output, ImageFormat::WebP)?;
    metrics::histogram!("avatar_processing_duration_secs").record(start.elapsed().as_secs_f64());

    // Upload image
    let data = output.into_inner();
    let sha1 = openssl::sha::sha1(&data);
    let filename = format!("images/avatar/{}.webp", hex::encode(sha1));
    let bytes = bytes::Bytes::from(data);
    let result = state.file_host.upload(bytes, &filename).await?;

    // Save url
    user.avatar_url = Some(result.public_url);
    user_dao.update_by_id(&mut user).await?;

    ok!(())
}
