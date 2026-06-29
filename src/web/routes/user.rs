use crate::db::user::{IUserDao, UserDao};
use crate::db::CrudDao;
use crate::search::user::UserDocument;
use crate::service::connection_account::{GenerateChallengeError, VerifyChallengeError};
use crate::service::errors::ServiceError;
use crate::service::follow::FollowError;
use crate::service::upload::ResizeType;
use crate::web::jwt::{Claims, OptionalClaims};
use crate::web::result::{CommonError, WebError, WebResult};
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/greet", get(greet))
        .route("/profile", get(get_profile))
        .route("/update_profile", post(update_profile))
        .route("/set_avatar", post(set_avatar).layer(DefaultBodyLimit::max(10 * 1024 * 1024)))
        .route("/search", get(search))
        // @since 260402 @experimental
        .nest("/connection", Router::new()
            .route("/list", get(connection_list))
            .route("/unlink", post(connection_unlink))
            .route("/set_visibility", post(connection_set_visibility))
            .route("/sync", post(connection_sync))
            .route("/generate_challenge", post(connection_generate_challenge))
            .route("/verify_challenge", post(connection_verify_challenge)),
        )
        // @since 260626 @experimental
        .route("/following", get(get_following))
        // @since 260626 @experimental
        .route("/followers", get(get_followers))
        // @since 260626 @experimental
        .route("/follow", post(follow))
        // @since 260626 @experimental
        .route("/unfollow", post(unfollow))
}

async fn greet() -> WebResult<&'static str> {
    ok!("Hello from Hachimi World!")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetProfileReq {
    pub uid: i64,
}

#[axum::debug_handler]
async fn get_profile(
    state: State<AppState>,
    claims: OptionalClaims,
    req: Query<GetProfileReq>,
) -> WebResult<service::user::PublicUserProfile> {
    let viewer_uid = claims.0.as_ref().map(|c| c.uid());
    let mut profiles = service::user::get_public_profile(state.redis_conn.clone(), &state.sql_pool, &[req.uid], viewer_uid).await?;

    let profile = profiles.remove(&req.uid)
        .ok_or_else(|| WebError::common("not_found", "User not found"))?;
    ok!(profile)
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
        follower_count: user.follower_count.unwrap_or(0),
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
        follower_count: user.follower_count.unwrap_or(0),
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
    pub hits: Vec<service::user::PublicUserProfile>,
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
    let users = service::user::get_public_profile(state.redis_conn.clone(), &state.sql_pool, &user_ids, None).await?
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionResp {
    pub items: Vec<ConnectionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionItem {
    pub r#type: String,
    pub id: String,
    pub name: String,
    pub public: bool,
}

async fn connection_list(
    claims: Claims,
    state: State<AppState>,
) -> WebResult<ConnectionResp> {
    let connections = service::connection_account::list_connections(
        &state.sql_pool, state.redis_conn.clone(),
        claims.uid(), false,
    ).await?;

    let mapped_items = connections.into_iter().map(|c| ConnectionItem {
        r#type: c.r#type,
        id: c.id,
        name: c.name,
        public: c.public,
    }).collect_vec();

    ok!(ConnectionResp {
        items: mapped_items
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionUnlinkReq {
    pub r#type: String,
}

async fn connection_unlink(
    claims: Claims,
    state: State<AppState>,
    req: Json<ConnectionUnlinkReq>,
) -> WebResult<()> {
    service::connection_account::unlink(&state.sql_pool, state.redis_conn.clone(), claims.uid(), &req.r#type).await?;
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSetVisibilityReq {
    pub r#type: String,
    pub visible: bool,
}

async fn connection_set_visibility(
    _claims: Claims,
    _req: Json<ConnectionSetVisibilityReq>,
) -> WebResult<()> {
    // TODO
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateChallengeReq {
    pub r#type: String,
    pub provider_account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateChallengeResp {
    pub challenge_id: String,
    pub challenge: String,
    pub provider_account_name: String,
}

async fn connection_generate_challenge(
    claims: Claims,
    state: State<AppState>,
    req: Json<GenerateChallengeReq>,
) -> WebResult<GenerateChallengeResp> {
    match service::connection_account::generate_challenge(
        &state.bili_client,
        state.redis_conn.clone(),
        claims.uid(),
        &req.r#type,
        &req.provider_account_id,
    ).await {
        Ok(challenge) => {
            ok!(GenerateChallengeResp {
                challenge_id: challenge.challenge_id,
                challenge: challenge.challenge,
                provider_account_name: challenge.provider_account_name
            })
        }
        Err(x) => match x {
            GenerateChallengeError::UnsupportedProviderType => err!("unsupported_provider_type", "{}", x.to_string()),
            GenerateChallengeError::ProviderApiError(_) => err!("provider_api_error", "{}", x.to_string()),
            GenerateChallengeError::InvalidProviderAccountId => err!("invalid_provider_account_id", "{}", x.to_string()),
            GenerateChallengeError::ProviderAccountNotFound => err!("provider_account_not_found", "{}", x.to_string()),
            GenerateChallengeError::Other(e) => Err(e)?,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyChallengeReq {
    pub challenge_id: String,
}

async fn connection_verify_challenge(
    claims: Claims,
    state: State<AppState>,
    req: Json<VerifyChallengeReq>,
) -> WebResult<()> {
    match service::connection_account::verify_challenge_and_link(
        &state.bili_client,
        &state.sql_pool,
        state.red_lock.clone(),
        state.redis_conn.clone(),
        claims.uid(),
        &req.challenge_id,
    ).await {
        Ok(_) => ok!(()),
        Err(e) => match e {
            VerifyChallengeError::ChallengeNotFound => err!("challenge_not_found", "{}", e.to_string()),
            VerifyChallengeError::ChallengeMismatch => err!("challenge_mismatch", "{}", e.to_string()),
            VerifyChallengeError::UnsupportedProviderType => err!("unsupported_provider_type", "{}", e.to_string()),
            VerifyChallengeError::AlreadyLinked => err!("already_linked", "{}", &e.to_string()),
            VerifyChallengeError::ProviderAccountNotFound => err!("provider_account_not_found", "{}", e.to_string()),
            VerifyChallengeError::ProviderApiError(_) => err!("provider_api_error", "{}", e.to_string()),
            VerifyChallengeError::Other(e) => Err(e)?,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSyncReq {
    r#type: String,
}

async fn connection_sync(
    claims: Claims,
    state: State<AppState>,
    req: Json<ConnectionSyncReq>,
) -> WebResult<()> {
    service::connection_account::sync(
        &state.bili_client,
        &state.sql_pool, claims.uid(),
        &req.r#type
    ).await?;
    ok!(())
}

// ─── Follow / Unfollow ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowReq {
    pub target_uid: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowResp {
    pub follower_count: i64,
}

async fn follow(
    claims: Claims,
    state: State<AppState>,
    req: Json<FollowReq>,
) -> WebResult<FollowResp> {
    let my_uid = claims.uid();
    // Check that current user is not banned
    let me = UserDao::get_by_id(&state.sql_pool, my_uid).await?
        .ok_or_else(|| common!("not_found", "User not found"))?;
    if me.is_banned {
        err!("user_banned", "You are banned and cannot follow users");
    }

    let follower_count = service::follow::follow_user(
        state.redis_conn.clone(),
        &state.sql_pool,
        my_uid,
        req.target_uid,
    ).await?;

    ok!(FollowResp { follower_count })
}

async fn unfollow(
    claims: Claims,
    state: State<AppState>,
    req: Json<FollowReq>,
) -> WebResult<FollowResp> {
    let my_uid = claims.uid();

    let follower_count = service::follow::unfollow_user(
        state.redis_conn.clone(),
        &state.sql_pool,
        my_uid,
        req.target_uid,
    ).await?;

    ok!(FollowResp { follower_count })
}

impl From<ServiceError<FollowError>> for WebError<CommonError> {
    fn from(err: ServiceError<FollowError>) -> Self {
        match err {
            ServiceError::BusinessError(fe) => {
                let (code, msg) = match fe {
                    FollowError::CannotFollowYourself => ("cannot_follow_self", "Cannot follow yourself"),
                    FollowError::CannotUnfollowYourself => ("cannot_unfollow_self", "Cannot unfollow yourself"),
                    FollowError::TargetUserNotFound => ("not_found", "User not found"),
                    FollowError::TargetUserBanned => ("user_banned", "Target user is banned"),
                };
                common!(code, "{}", msg)
            }
            ServiceError::Other(e) => WebError::Internal(e),
        }
    }
}

// ─── Following / Followers Lists ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowingListReq {
    pub after: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 { 20 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowingListResp {
    pub items: Vec<service::follow::FollowingItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

async fn get_following(
    claims: Claims,
    state: State<AppState>,
    req: Query<FollowingListReq>,
) -> WebResult<FollowingListResp> {
    let limit = req.limit.min(50).max(1);
    let (items, next_cursor) = service::follow::get_following(
        state.redis_conn.clone(),
        &state.sql_pool,
        claims.uid(),
        req.after.as_deref(),
        limit,
    ).await?;

    ok!(FollowingListResp { items, next_cursor })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowersListResp {
    pub items: Vec<service::follow::FollowerItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

async fn get_followers(
    claims: Claims,
    state: State<AppState>,
    req: Query<FollowingListReq>,
) -> WebResult<FollowersListResp> {
    let limit = req.limit.min(50).max(1);
    let (items, next_cursor) = service::follow::get_followers(
        state.redis_conn.clone(),
        &state.sql_pool,
        claims.uid(),
        req.after.as_deref(),
        limit,
    ).await?;

    ok!(FollowersListResp { items, next_cursor })
}