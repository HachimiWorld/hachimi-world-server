use crate::db::sitemap_generation::{SitemapGeneration, SitemapGenerationDao};
use crate::db::CrudDao;
use crate::service::sitemap::{self, SitemapCfg};
use crate::web::jwt::AuthError;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{err, ok};
use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::routing::{get, post};
use axum::{RequestPartsExt, Router};
use axum_extra::headers::authorization::Bearer;
use axum_extra::headers::Authorization;
use axum_extra::TypedHeader;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// @since 261003
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/generate", post(generate))
        .route("/generation/list", get(generation_list))
}

/// Requires `Authorization: Bearer <sitemap.trigger_token>`. Rejects everything when the
/// `sitemap` config is absent.
pub struct SitemapClaims {
    cfg: SitemapCfg,
}

impl FromRequestParts<AppState> for SitemapClaims {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let TypedHeader(Authorization(bearer)) = parts.extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| AuthError::MissingCredentials)?;
        let cfg = state.config.get_and_parse::<SitemapCfg>("sitemap").map_err(|_| AuthError::InvalidToken)?;
        if bearer.token() != cfg.trigger_token {
            return Err(AuthError::InvalidToken);
        }
        Ok(SitemapClaims { cfg })
    }
}

/// @since 261003
#[derive(Debug, Serialize, Deserialize)]
pub struct SitemapGenerationItem {
    pub id: i64,
    /// `schedule` or `manual`.
    pub trigger_type: String,
    /// `running`, `success` or `failure`. `running` for a past run means the server stopped during it.
    pub status: String,
    pub song_count: Option<i32>,
    pub file_count: Option<i32>,
    pub error: Option<String>,
    pub start_time: DateTime<Utc>,
    pub finish_time: Option<DateTime<Utc>>,
}

impl From<SitemapGeneration> for SitemapGenerationItem {
    fn from(x: SitemapGeneration) -> Self {
        Self {
            id: x.id,
            trigger_type: x.trigger_type,
            status: x.status,
            song_count: x.song_count,
            file_count: x.file_count,
            error: x.error,
            start_time: x.start_time,
            finish_time: x.finish_time,
        }
    }
}

/// Generates and uploads the sitemaps now and returns the record. A failed generation is still
/// an ok response, with `status` `failure`.
async fn generate(claims: SitemapClaims, state: State<AppState>) -> WebResult<SitemapGenerationItem> {
    match sitemap::run(&state, &claims.cfg, sitemap::TRIGGER_MANUAL).await? {
        Some(generation) => ok!(generation.into()),
        None => err!("generation_running", "A sitemap generation is already running"),
    }
}

/// @since 261003
#[derive(Debug, Serialize, Deserialize)]
pub struct GenerationListReq {
    pub page_index: i64,
    /// 1..=50
    pub page_size: i64,
}

/// @since 261003
#[derive(Debug, Serialize, Deserialize)]
pub struct GenerationListResp {
    /// Most recent first.
    pub data: Vec<SitemapGenerationItem>,
    pub page_index: i64,
    pub page_size: i64,
    pub total: i64,
}

async fn generation_list(_claims: SitemapClaims, state: State<AppState>, req: Query<GenerationListReq>) -> WebResult<GenerationListResp> {
    let page_index = req.page_index.max(0);
    let page_size = req.page_size.clamp(1, 50);
    let data = SitemapGenerationDao::page(&state.sql_pool, page_index, page_size).await?
        .into_iter()
        .map(SitemapGenerationItem::from)
        .collect();
    let total = SitemapGenerationDao::count(&state.sql_pool).await?;
    ok!(GenerationListResp { data, page_index, page_size, total })
}
