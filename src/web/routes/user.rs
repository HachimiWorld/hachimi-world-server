use crate::db::user::{IUserDao, UserDao};
use crate::db::CrudDao;
use crate::search::user::UserDocument;
use crate::service::upload::ResizeType;
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{common, err, ok, search, service};
use anyhow::Context;
use async_backtrace::framed;
use axum::extract::{DefaultBodyLimit, Multipart, Query};
use axum::routing::post;
use axum::{extract::State, routing::get, Json, Router};
use chrono::Utc;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/greet", get(greet))
        .route("/profile", get(get_profile))
        .route("/update_profile", post(update_profile))
        .route("/set_avatar", post(set_avatar).layer(DefaultBodyLimit::max(10 * 1024 * 1024)))
        .route("/search", get(search))
}

async fn greet() -> WebResult<&'static str> {
    ok!("Hello from Hachimi World!")
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct GetProfileReq {
    pub uid: i64,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PublicUserProfile {
    pub uid: i64,
    pub username: String,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub gender: Option<i32>,
    pub is_banned: bool,
}

async fn get_profile(
    state: State<AppState>,
    req: Query<GetProfileReq>,
) -> WebResult<PublicUserProfile> {
    // Fetch user from db
    let user = if let Some(x) = UserDao::get_by_id(&state.sql_pool, req.uid).await? {
        x
    } else {
        err!("not_found", "User not found")
    };

    let mapped = PublicUserProfile {
        uid: user.id,
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

    if let Some(user) = UserDao::get_by_username(&state.sql_pool, &req.username).await? {
        if user.id != claims.uid() {
            err!("username_exists", "Username already exists");
        }
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
    let mut user = if let Some(x) = UserDao::get_by_id(&state.sql_pool, claims.uid()).await? {
        x
    } else {
        err!("not_found", "User not found")
    };
    user.username = req.username.clone();
    user.gender = req.gender;
    user.bio = req.bio.clone();
    user.update_time = Utc::now();
    UserDao::update_by_id(&state.sql_pool, &user).await?;
    search::user::update_user_document(&state.meilisearch, UserDocument {
        id: user.id,
        avatar_url: user.avatar_url,
        name: user.username,
        follower_count: 0,
    }).await?;

    ok!(())
}

#[framed]
async fn set_avatar(
    claims: Claims,
    state: State<AppState>,
    mut multipart: Multipart,
) -> WebResult<()> {
    // TODO[opt]: Limit access rate
    let mut user = if let Some(x) = UserDao::get_by_id(&state.sql_pool, claims.uid()).await? {
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
    let webp = service::upload::scale_down_to_webp(256, 256, bytes.clone(), ResizeType::Crop, 80f32)
        .map_err(|_| common!("invalid_image", "The image might not be supported"))?;

    // Upload image
    let sha1 = openssl::sha::sha1(&webp);
    let filename = format!("images/avatar/{}.webp", hex::encode(sha1));

    metrics::histogram!("avatar_processing_duration_secs").record(start.elapsed().as_secs_f64());
    let result = state.file_host.upload(webp.into(), &filename).await?;

    // Save url
    user.avatar_url = Some(result.public_url);
    UserDao::update_by_id(&state.sql_pool, &mut user).await?;

    search::user::update_user_document(&state.meilisearch, UserDocument {
        id: user.id,
        avatar_url: user.avatar_url,
        name: user.username,
        follower_count: 0,
    }).await?;

    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchReq {
    pub q: String,
    pub page: u32,
    #[serde(default = "default_search_size")]
    pub size: u32,
}

fn default_search_size() -> u32 { 20 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResp {
    pub hits: Vec<PublicUserProfile>,
    pub query: String,
    pub processing_time_ms: u64,
    pub total_hits: Option<usize>,
    pub limit: usize,
    pub offset: usize,
}

async fn search(
    state: State<AppState>,
    req: Query<SearchReq>,
) -> WebResult<SearchResp> {
    if req.size > 50 { err!("invalid_size", "Size must be less than 50"); }

    let offset = req.page * req.size;
    let result = search::user::search_users(
        &state.meilisearch,
        &req.q,
        Some(req.size as usize),
        Some(offset as usize),
    ).await?;

    let user_ids: Vec<i64> = result.hits.iter().map(|u| u.id).collect();
    let users = service::user::get_public_profile(state.redis_conn.clone(), &state.sql_pool, &user_ids).await?
        .into_iter().map(|(_, v)| v)
        .collect_vec();

    ok!(SearchResp {
        hits: users,
        query: result.query,
        processing_time_ms: result.processing_time_ms,
        total_hits: result.hits_info.total_hits,
        limit: result.hits_info.limit,
        offset: result.hits_info.offset,
    })
}