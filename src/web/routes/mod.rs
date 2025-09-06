pub mod user;
pub mod auth;
pub mod song;
pub mod playlist;
pub mod version;
pub mod play_history;
pub mod publish;

use axum::Router;
use crate::web::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/user", user::router())
        .nest("/song", song::router())
        .nest("/play_history", play_history::router())
        .nest("/playlist", playlist::router())
        .nest("/version", version::router())
        .nest("/publish", publish::router())
}