use crate::db::user::UserDao;
use crate::db::CrudDao;
use crate::web::jwt::Claims;
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::state::AppState;
use crate::{common, err, ok, service};
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::Router;
use chrono::{DateTime, Utc};
use futures::future::ok;
use redis::AsyncTypedCommands;
use serde::{Deserialize, Serialize};
use serde_json::Error;
use tracing::warn;
use crate::db::song::{Song, SongDao, SongOriginInfo, SongProductionCrew};
use crate::db::song_publishing_review::{ISongPublishingReviewDao, SongPublishingReview, SongPublishingReviewDao};
use crate::db::song_tag::SongTag;
use crate::web::routes::song::{ExternalLink, TagItem};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/review/page", get(page))
        .route("/review/page_contributor", get(page_contributor))
        .route("/review/approve", post(review_approve))
        .route("/review/reject", post(review_reject))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageReq {
    pub page_index: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageResp {
    pub data: Vec<SongPublishReviewBrief>,
    pub page_index: i64,
    pub page_size: i64,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongPublishReviewBrief {
    pub review_id: i64,
    pub title: String,
    pub subtitle: String,
    pub artist: String,
    pub cover_url: String,
    pub submit_time: DateTime<Utc>,
    pub review_time: Option<DateTime<Utc>>,
    pub review_comment: Option<String>,
    pub status: i32,
}

impl TryFrom<SongPublishingReview> for SongPublishReviewBrief {
    type Error = serde_json::Error;
    fn try_from(value: SongPublishingReview) -> Result<Self, Self::Error> {
        serde_json::from_value::<SongPublishReviewData>(value.data).map(|decode|
            SongPublishReviewBrief {
                review_id: value.id,
                title: decode.song_info.title,
                subtitle: decode.song_info.subtitle,
                artist: decode.song_info.artist,
                cover_url: decode.song_info.cover_art_url,
                submit_time: value.submit_time,
                review_time: value.review_time,
                review_comment: value.review_comment,
                status: value.status,
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongPublishReviewData {
    pub song_info: Song,
    pub song_origin_infos: Vec<SongOriginInfo>,
    pub song_production_crew: Vec<SongProductionCrew>,
    pub song_tags: Vec<SongTag>,
    pub song_external_links: Vec<ExternalLink>,
}

async fn page(
    claims: Claims,
    state: State<AppState>,
    req: Query<PageReq>,
) -> WebResult<PageResp> {
    if req.page_size > 50 {
        err!("page_size_exceeded", "Page size too large");
    }
    let result = SongPublishingReviewDao::page_by_user(&state.sql_pool, claims.uid(), req.page_index, req.page_size).await?;
    let brief: Vec<_> = result.into_iter().map(|x| {
        match SongPublishReviewBrief::try_from(x.clone()) {
            Ok(v) => {
                v
            }
            Err(err) => {
                warn!("Error during decoding song publish review data: {:?}", err);
                SongPublishReviewBrief {
                    review_id: x.id,
                    title: "Unknown".to_string(),
                    subtitle: "Unknown".to_string(),
                    artist: "Unknown".to_string(),
                    cover_url: "Unknown".to_string(),
                    submit_time: x.submit_time,
                    review_time: x.review_time,
                    review_comment: x.review_comment,
                    status: x.status,
                }
            }
        }
    }).collect();
    let count = SongPublishingReviewDao::count_by_user(&state.sql_pool, claims.uid()).await?;
    let resp = PageResp {
        data: brief,
        page_index: req.page_index,
        page_size: req.page_size,
        total: count,
    };
    ok!(resp)
}

async fn page_contributor(
    claims: Claims,
    state: State<AppState>,
    req: Query<PageReq>,
) -> WebResult<PageResp> {
    if req.page_size > 50 {
        err!("page_size_exceeded", "Page size too large");
    }
    ensure_contributor(state.clone().0, claims.uid()).await?;

    let result = SongPublishingReviewDao::page(&state.sql_pool, req.page_index, req.page_size).await?;
    let brief: Vec<_> = result.into_iter().map(|x| {
        match SongPublishReviewBrief::try_from(x.clone()) {
            Ok(v) => {
                v
            }
            Err(err) => {
                warn!("Error during decoding song publish review data: {:?}", err);
                SongPublishReviewBrief {
                    review_id: x.id,
                    title: "Unknown".to_string(),
                    subtitle: "Unknown".to_string(),
                    artist: "Unknown".to_string(),
                    cover_url: "Unknown".to_string(),
                    submit_time: x.submit_time,
                    review_time: x.review_time,
                    review_comment: x.review_comment,
                    status: x.status,
                }
            }
        }
    }).collect();
    let count = SongPublishingReviewDao::count(&state.sql_pool).await?;
    let resp = PageResp {
        data: brief,
        page_index: req.page_index,
        page_size: req.page_size,
        total: count,
    };
    ok!(resp)
}

async fn review_approve(
    claims: Claims,
    state: State<AppState>,
) -> WebResult<()> {
    ensure_contributor(state.clone().0, claims.uid()).await?;

    /*let mut tx = state.sql_pool.begin().await?;
    let song_id = SongDao::insert(&mut *tx, &song).await?;
    SongDao::update_song_origin_info(&mut tx, song_id, &song_origin_infos).await?;
    SongDao::update_song_production_crew(&mut tx, song_id, &production_crew).await?;

    let tag_ids = tags.iter().map(|x| x.id).collect();
    SongDao::update_song_tags(&mut tx, song_id, tag_ids).await?;
    tx.commit().await?;

    // Write behind, data consistence is not guaranteed.
    search::song::add_song_document(
        state.meilisearch.as_ref(),
        song_id,
        &song,
        &production_crew,
        &song_origin_infos,
        &tags,
    ).await?;
    service::recommend_v2::notify_update(song_id, &state.redis_conn).await?;*/

    todo!();
}

async fn review_reject(
    claims: Claims,
    state: State<AppState>,
) -> WebResult<()> {
    ensure_contributor(state.0.clone(), claims.uid()).await?;
    todo!()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityCfg {
    pub contributors: Vec<String>,
}

async fn ensure_contributor(
    mut state: AppState,
    uid: i64,
) -> Result<(), WebError<CommonError>> {
    let config = state.config;
    let pool = &state.sql_pool;
    let redis = &mut state.redis_conn;
    let contributors = redis.get("contributors").await?;
    if let Some(contributors) = contributors {
        let contributors: Vec<i64> = serde_json::from_str(&contributors)?;
        if contributors.contains(&uid) {
            Ok(())
        } else {
            Err(common!("permission_denied", "You are not a contributor"))
        }
    } else {
        // TODO: Get from github repository
        let cfg: CommunityCfg = config.get_and_parse("community")?;
        let mut user_ids = Vec::new();
        for x in cfg.contributors {
            if let Some(user) = UserDao::get_by_id(pool, uid).await? {
                user_ids.push(user.id);
            } else {
                warn!("Contributor {} was configured but not found in database", x);
            }
        }
        redis.set("contributors", serde_json::to_string(&user_ids)?).await?;
        if user_ids.contains(&uid) {
            Ok(())
        } else {
            Err(common!("permission_denied", "You are not a contributor"))
        }
    }
}

fn map() {}