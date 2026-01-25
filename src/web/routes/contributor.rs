use crate::ok;
use crate::service::contributor;
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use axum::extract::State;
use axum::Router;
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        // @since 260125
        .route("/check", axum::routing::get(check_contributor))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckContributorResp {
    pub is_contributor: bool,
}

async fn check_contributor(
    claims: Claims,
    state: State<AppState>,
) -> WebResult<CheckContributorResp> {
    let result = contributor::check_contributor(
        &state.config,
        state.redis_conn.clone(),
        &state.red_lock,
        &state.sql_pool,
        claims.uid()
    ).await?;
    ok!(CheckContributorResp {
        is_contributor: result,
    })
}