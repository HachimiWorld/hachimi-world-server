pub mod user;
pub mod auth;
pub mod song;
pub mod playlist;
// pub mod follow;
// pub mod file;

use axum::Router;
use crate::web::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/user", user::router())
        .nest("/song", song::router())
        .nest("/playlist", playlist::router())
}