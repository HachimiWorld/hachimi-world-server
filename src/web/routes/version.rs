use crate::db::version::{Version, VersionDao};
use crate::db::CrudDao;
use crate::web::jwt::PublishVersionClaims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{err, ok};
use aide::axum::routing::{get, post};
use aide::axum::ApiRouter;
use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use redis::aio::ConnectionManager;
use redis::{AsyncTypedCommands, HashFieldExpirationOptions, SetExpiry};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

pub fn router() -> ApiRouter<AppState> {
    ApiRouter::new()
        .api_route("/server", get(server))
        .api_route("/latest", get(latest_version))
        .api_route("/latest_batch", post(latest_version_batch))
        .api_route("/page", get(page_versions))
        .api_route("/publish", post(publish_version))
        .api_route("/delete", post(delete_version))
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ServerVersion {
    pub version: i32,
    pub min_version: i32,
}

async fn server() -> WebResult<ServerVersion> {
    ok!(ServerVersion {
        version: 261031,
        min_version: 250905
    })
}


#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LatestVersionReq {
    pub variant: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LatestVersionResp {
    pub version_name: String,
    pub version_number: i32,
    pub changelog: String,
    pub variant: String,
    pub url: String,
    /// Package size in bytes. `None` for versions published without it.
    /// @since 261001
    pub size: Option<i64>,
    /// Lowercase hex SHA-256 of the package. `None` for versions published without it.
    /// @since 261001
    pub sha256: Option<String>,
    pub release_time: DateTime<Utc>,
}

impl From<Version> for LatestVersionResp {
    fn from(v: Version) -> Self {
        Self {
            version_name: v.version_name,
            version_number: v.version_number,
            changelog: v.changelog,
            variant: v.variant,
            url: v.url,
            size: v.size,
            sha256: v.sha256,
            release_time: v.release_time,
        }
    }
}

async fn latest_version(state: State<AppState>, req: Query<LatestVersionReq>) -> WebResult<Option<LatestVersionResp>> {
    let version = get_from_cache_or_db(&state.sql_pool, state.redis_conn.clone(), &req.variant).await?;
    ok!(version.map(LatestVersionResp::from))
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LatestVersionBatchReq {
    pub variants: Vec<String>,
}
async fn latest_version_batch(state: State<AppState>, req: Json<LatestVersionBatchReq>) -> WebResult<Vec<LatestVersionResp>> {
    if req.variants.len() > 16 {
        err!("bad_request", "Variants must be less than 16")
    }
    let mut result = vec![];
    for x in req.variants.iter() {
        let version = get_from_cache_or_db(&state.sql_pool, state.redis_conn.clone(), x).await?;
        if let Some(version) = version {
            result.push(version.into())
        }
    }
    ok!(result)
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PageVersionsReq {
    pub variant: Option<String>,
    pub page_index: i64,
    pub page_size: i64,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PageVersionsResp {
    pub data: Vec<LatestVersionResp>,
    pub page_index: i64,
    pub page_size: i64,
    pub total: i64,
}

async fn page_versions(state: State<AppState>, req: Query<PageVersionsReq>) -> WebResult<PageVersionsResp> {
    let page_index = req.page_index.max(0);
    let page_size = req.page_size.clamp(1, 50);

    // Scheduled (future) releases stay hidden until their release time, same as `/latest`
    let now = Utc::now();
    let variant = req.variant.as_deref();
    let versions = VersionDao::page_released(&state.sql_pool, variant, now, page_index, page_size).await?;
    let total = VersionDao::count_released(&state.sql_pool, variant, now).await?;

    let data = versions
        .into_iter()
        .map(LatestVersionResp::from)
        .collect();

    ok!(PageVersionsResp {
        data,
        page_index,
        page_size,
        total,
    })
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PublishVersionReq {
    pub version_name: String,
    pub version_number: i32,
    pub changelog: String,
    pub variant: String,
    pub url: String,
    /// Package size in bytes, must be positive.
    /// @since 261001
    #[serde(default)]
    pub size: Option<i64>,
    /// Hex SHA-256 of the package (64 characters, stored lowercase).
    /// @since 261001
    #[serde(default)]
    pub sha256: Option<String>,
    pub release_time: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PublishVersionResp {
    pub id: i64,
}

async fn publish_version(
    _claims: PublishVersionClaims,
    state: State<AppState>,
    req: Json<PublishVersionReq>,
) -> WebResult<PublishVersionResp> {
    if let Some(size) = req.size && size <= 0 {
        err!("invalid_size", "Size must be positive")
    }
    let sha256 = match &req.sha256 {
        Some(hash) if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) => Some(hash.to_ascii_lowercase()),
        Some(_) => err!("invalid_sha256", "SHA-256 must be 64 hex characters"),
        None => None,
    };

    let entity = Version {
        id: 0,
        version_name: req.version_name.clone(),
        version_number: req.version_number,
        changelog: req.changelog.clone(),
        variant: req.variant.clone(),
        url: req.url.clone(),
        size: req.size,
        sha256,
        release_time: req.release_time,
        create_time: Utc::now(),
        update_time: Utc::now(),
    };

    let id = VersionDao::insert(&state.sql_pool, &entity).await?;
    clear_cache(state.redis_conn.clone()).await?;
    ok!(PublishVersionResp { id })
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DeleteVersionReq {
    pub id: i64,
}

async fn delete_version(
    _claims: PublishVersionClaims,
    state: State<AppState>,
    req: Json<DeleteVersionReq>,
) -> WebResult<()> {
    VersionDao::delete_by_id(&state.sql_pool, req.id).await?;
    clear_cache(state.redis_conn.clone()).await?;
    ok!(())
}

async fn get_from_cache_or_db(
    sql_pool: &PgPool,
    mut redis: ConnectionManager,
    variant: &str,
) -> anyhow::Result<Option<Version>> {
    let data = redis.hget("version:latest", variant).await?;
    let result = if let Some(data) = &data &&
        let Ok(v) = serde_json::from_str::<Option<Version>>(data) {
        v
    } else {
        let version = VersionDao::get_latest_version(sql_pool, &variant, Utc::now()).await?;
        redis.hset_ex(
            "version:latest",
            &HashFieldExpirationOptions::default().set_expiration(SetExpiry::EX(60 * 60)),
            &[(variant, serde_json::to_string(&version)?)]
        ).await?;
        version
    };
    Ok(result)
}

async fn clear_cache(mut redis: ConnectionManager) -> anyhow::Result<()> {
    redis.del("version:latest").await?;
    Ok(())
}