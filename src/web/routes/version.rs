use crate::web::state::AppState;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use crate::ok;
use crate::web::result::WebResult;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/server", get(server))
}

#[derive(Serialize)]
pub struct ServerVersion {
    pub version: i32,
    pub min_version: i32
}

async fn server() -> WebResult<ServerVersion> {
    ok!(ServerVersion {
        version: 250905,
        min_version: 250905
    })
}