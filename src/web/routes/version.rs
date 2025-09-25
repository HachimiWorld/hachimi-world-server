use crate::db::version::VersionDao;
use crate::db::CrudDao;
use crate::web::jwt::PublishVersionClaims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{db, ok};
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/server", get(server))
        .route("/latest", get(latest_version))
        .route("/publish", post(publish_version))
        .route("/delete", post(delete_version))
}

#[derive(Serialize)]
pub struct ServerVersion {
    pub version: i32,
    pub min_version: i32
}

async fn server() -> WebResult<ServerVersion> {
    ok!(ServerVersion {
        version: 250925,
        min_version: 250905
    })
}


#[derive(Debug, Serialize, Deserialize)]
pub struct LatestVersionReq {
    pub variant: String
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LatestVersionResp {
    pub version_name: String,
    pub version_number: i32,
    pub changelog: String,
    pub variant: String,
    pub url: String,
    pub release_time: DateTime<Utc>
}

async fn latest_version(state: State<AppState>, req: Query<LatestVersionReq>) -> WebResult<Option<LatestVersionResp>>{
    let version = VersionDao::get_latest_version(&state.sql_pool, &req.variant, Utc::now()).await?;
    if let Some(version) = version {
        let result = LatestVersionResp {
            variant: version.variant,
            version_name: version.version_name,
            version_number: version.version_number,
            changelog: version.changelog,
            url: version.url,
            release_time: version.release_time
        };
        ok!(Some(result))
    } else {
        ok!(None)
    }
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
    req: Json<PublishVersionReq>
) -> WebResult<PublishVersionResp> {
    let entity = db::version::Version {
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
    ok!(PublishVersionResp { id })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteVersionReq {
    pub id: i64
}

async fn delete_version(
    _claims: PublishVersionClaims,
    state: State<AppState>,
    req: Json<DeleteVersionReq>
) -> WebResult<()> {
    VersionDao::delete_by_id(&state.sql_pool, req.id).await?;
    ok!(())
}