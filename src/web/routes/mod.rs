pub mod user;
pub mod auth;
mod follow;
mod file;

use axum::Router;
use crate::web::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/user", user::router())
}