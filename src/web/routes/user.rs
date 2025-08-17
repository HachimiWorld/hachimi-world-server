use crate::db::CrudDao;
use crate::db::user::{IUserDao, UserDao};
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::result::{WebError, WebResponse};
use crate::web::state::AppState;
use crate::{err, ok};
use axum::routing::post;
use axum::{Json, Router, extract::State, routing::get};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/greet", get(greet))
        .route("/profile", post(get_profile))
        .route("/update_profile", post(update_profile))
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
