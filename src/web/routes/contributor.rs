use crate::ok;
use crate::service::contributor;
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use aide::axum::routing::get_with;
use crate::web::routes::since;
use aide::axum::ApiRouter;
use axum::extract::State;
use serde::{Deserialize, Serialize};

pub fn router() -> ApiRouter<AppState> {
    ApiRouter::new()
        .api_route("/check", get_with(check_contributor, since("260125")))
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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