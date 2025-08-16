pub mod user;
pub mod auth;

use axum::Router;
use crate::web::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/user", user::router())
}