use crate::db::version::{Version, VersionDao};
use crate::db::CrudDao;
use crate::web::jwt::PublishVersionClaims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{err, ok};
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use redis::aio::ConnectionManager;
use redis::{AsyncTypedCommands, HashFieldExpirationOptions, SetExpiry};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/server", get(server))
        .route("/latest", get(latest_version))
        .route("/latest_batch", post(latest_version_batch))
        .route("/publish", post(publish_version))
        .route("/delete", post(delete_version))
}

#[derive(Serialize)]
pub struct ServerVersion {
    pub version: i32,
    pub min_version: i32,
}

async fn server() -> WebResult<ServerVersion> {
    ok!(ServerVersion {
        version: 251119,
        min_version: 250905
    })
}


#[derive(Debug, Serialize, Deserialize)]
pub struct LatestVersionReq {
    pub variant: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LatestVersionResp {
    pub version_name: String,
    pub version_number: i32,
    pub changelog: String,
    pub variant: String,
    pub url: String,
    pub release_time: DateTime<Utc>,
}

async fn latest_version(state: State<AppState>, req: Query<LatestVersionReq>) -> WebResult<Option<LatestVersionResp>> {
    let version = get_from_cache_or_db(&state.sql_pool, state.redis_conn.clone(), &req.variant).await?;
    if let Some(version) = version {
        let result = LatestVersionResp {
            variant: version.variant,
            version_name: version.version_name,
            version_number: version.version_number,
            changelog: version.changelog,
            url: version.url,
            release_time: version.release_time,
        };
        ok!(Some(result))
    } else {
        ok!(None)
    }
}

#[derive(Debug, Serialize, Deserialize)]
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
            result.push(LatestVersionResp {
                variant: version.variant,
                version_name: version.version_name,
                version_number: version.version_number,
                changelog: version.changelog,
                url: version.url,
                release_time: version.release_time,
            })
        }
    }
    ok!(result)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublishVersionReq {
    pub version_name: String,
    pub version_number: i32,
    pub changelog: String,
    pub variant: String,
    pub url: String,
    pub release_time: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PublishVersionResp {
    pub id: i64,
}

async fn publish_version(
    _claims: PublishVersionClaims,
    state: State<AppState>,
    req: Json<PublishVersionReq>,
) -> WebResult<PublishVersionResp> {
    let entity = Version {
        id: 0,
        version_name: req.version_name.clone(),
        version_number: req.version_number,
        changelog: req.changelog.clone(),
        variant: req.variant.clone(),
        url: req.url.clone(),
        release_time: req.release_time,
        create_time: Utc::now(),
        update_time: Utc::now(),
    };

    let id = VersionDao::insert(&state.sql_pool, &entity).await?;
    clear_cache(state.redis_conn.clone()).await?;
    ok!(PublishVersionResp { id })
}

#[derive(Debug, Serialize, Deserialize)]
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