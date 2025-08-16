use crate::web::result::WebResponse;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{err, ok};
use axum::{
    extract::{Request, State},
    middleware,
    routing::{get, put},
    Json, Router,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/greet", get(greet))
        // .route("/profile", get(get_profile).put(update_profile))
}

async fn greet() -> WebResult<&'static str> {
    ok!("Hello from Hachimi World!")
}

/*async fn get_profile(
    State(app_state): State<AppState>,
    request: Request,
) -> WebResult<PublicUser> {
    let claims = request.require_user()
        .map_err(|_| crate::web::result::WebError::common("UNAUTHORIZED", "Authentication required"))?;

    // Fetch user from db
    let user = sqlx::query_as::<_, crate::models::User>(
        "SELECT * FROM users WHERE id = $1 AND is_active = TRUE"
    )
    .bind(claims.user_id)
    .fetch_optional(&app_state.sql_pool)
    .await?;

    let user = user.ok_or_else(|| crate::web::result::WebError::common("USER_NOT_FOUND", "User not found"))?;

    ok!(user.into())
}

async fn update_profile(
    State(app_state): State<AppState>,
    Json(update_request): Json<UpdateProfileRequest>,
    request: Request,
) -> WebResult<PublicUser> {
    let claims = request.require_user()
        .map_err(|_| crate::web::result::WebError::common("UNAUTHORIZED", "Authentication required"))?;

    // Validate input
    if let Some(ref nickname) = update_request.nickname {
        if nickname.len() > 100 {
            err!("INVALID_NICKNAME", "Nickname must be 100 characters or less");
        }
    }

    if let Some(ref bio) = update_request.bio {
        if bio.len() > 500 {
            err!("INVALID_BIO", "Bio must be 500 characters or less");
        }
    }

    if let Some(ref gender) = update_request.gender {
        if !["male", "female", "secret"].contains(&gender.as_str()) {
            err!("INVALID_GENDER", "Gender must be 'male', 'female', or 'secret'");
        }
    }

    // Update user profile
    let updated_user = sqlx::query_as::<_, crate::models::User>(
        r#"
        UPDATE users 
        SET nickname = COALESCE($2, nickname),
            bio = COALESCE($3, bio),
            gender = COALESCE($4, gender),
            updated_at = NOW()
        WHERE id = $1 AND is_active = TRUE
        RETURNING *
        "#,
    )
    .bind(claims.user_id)
    .bind(&update_request.nickname)
    .bind(&update_request.bio)
    .bind(&update_request.gender)
    .fetch_optional(&app_state.sql_pool)
    .await?;

    let user = updated_user.ok_or_else(|| crate::web::result::WebError::common("USER_NOT_FOUND", "User not found"))?;

    ok!(user.into())
}*/