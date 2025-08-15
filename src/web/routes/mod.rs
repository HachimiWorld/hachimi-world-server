use axum::Router;
use crate::web::state::AppState;

pub fn router() -> axum::Router<AppState> {
    Router::new()
}