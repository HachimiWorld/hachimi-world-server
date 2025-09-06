use std::collections::HashMap;
use anyhow::Context;
use crate::db::user::UserDao;
use crate::db::CrudDao;
use crate::web::jwt::Claims;
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::state::AppState;
use crate::{common, err, ok, search, service};
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use redis::AsyncTypedCommands;
use serde::{Deserialize, Serialize};
use tracing::warn;
use crate::db::song::{Song, SongDao, SongOriginInfo, SongProductionCrew};
use crate::db::song_publishing_review::{ISongPublishingReviewDao, SongPublishingReview, SongPublishingReviewDao};
use crate::db::song_tag::SongTag;
use crate::util::IsBlank;
use crate::web::routes::auth::EmailConfig;
use crate::web::routes::song::{CreationTypeInfo, ExternalLink, TagItem};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/review/page", get(page))
        .route("/review/page_contributor", get(page_contributor))
        .route("/review/detail", get(detail))
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
    pub display_id: String,
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
        serde_json::from_value::<InternalSongPublishReviewData>(value.data).map(|decode|
            SongPublishReviewBrief {
                review_id: value.id,
                display_id: decode.song_info.display_id,
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
pub struct InternalSongPublishReviewData {
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
                    display_id: "Unknown".to_string(),
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
                    display_id: "Unknown".to_string(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailReq {
    pub review_id: i64,
}

pub type DetailResp = PublishSongPublishReviewData;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishSongPublishReviewData {
    pub review_id: i64,
    pub submit_time: DateTime<Utc>,
    pub review_time: Option<DateTime<Utc>>,
    pub review_comment: Option<String>,
    pub status: i32,
    pub display_id: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub duration_seconds: i32,
    pub lyrics: String,
    pub uploader_uid: i64,
    pub uploader_name: String,
    pub audio_url: String,
    pub cover_url: String,
    pub tags: Vec<TagItem>,
    pub production_crew: Vec<SongProductionCrew>,
    pub creation_type: i32,
    pub origin_infos: Vec<CreationTypeInfo>,
}

async fn detail(
    claims: Claims,
    state: State<AppState>,
    req: Query<DetailReq>,
) -> WebResult<DetailResp> {
    ensure_contributor(state.clone().0, claims.uid()).await?;
    let review = SongPublishingReviewDao::get_by_id(&state.sql_pool, req.review_id).await?;
    if let Some(review) = review {
        let data = serde_json::from_value::<InternalSongPublishReviewData>(review.data)
            .with_context(|| format!("Error during decoding song publish review({}) data", review.id))?;

        let uploader_name = UserDao::get_by_id(&state.sql_pool, data.song_info.uploader_uid).await?
            .map(|x| x.username)
            .unwrap_or_else(|| {
                warn!("User {} not found during compose review({}) detail data", data.song_info.uploader_uid, review.id);
                "Invalid".to_string()
            });


        let mut id_display_map = HashMap::new();

        for x in &data.song_origin_infos {
            match SongDao::get_by_id(&state.sql_pool, x.song_id).await? {
                Some(y) => {
                    id_display_map.insert(x.id, y.display_id);
                }
                None => {
                    // TODO: Consider to use other way to indicate the song was deleted
                    id_display_map.insert(x.id, "deleted".to_string());
                }
            }
        }

        let origin_infos_mapped = data.song_origin_infos.into_iter().map(|x| {
            let id = x.origin_song_id;
            CreationTypeInfo::from_song_origin_info(x, id.and_then(|x| id_display_map.get(&x).cloned()))
        }).collect();

        let result = PublishSongPublishReviewData {
            review_id: review.id,
            submit_time: review.submit_time,
            review_time: review.review_time,
            review_comment: review.review_comment,
            status: review.status,
            display_id: data.song_info.display_id,
            title: data.song_info.title,
            subtitle: data.song_info.subtitle,
            description: data.song_info.description,
            duration_seconds: data.song_info.duration_seconds,
            tags: data.song_tags.into_iter().map(|x| TagItem {
                id: x.id,
                name: x.name,
                description: x.description,
            }).collect(),
            lyrics: data.song_info.lyrics,
            audio_url: data.song_info.file_url,
            cover_url: data.song_info.cover_art_url,
            production_crew: data.song_production_crew,
            creation_type: data.song_info.creation_type,
            origin_infos: origin_infos_mapped,
            uploader_uid: data.song_info.uploader_uid,
            uploader_name: uploader_name,
        };
        ok!(result)
    } else {
        err!("not_found", "Review not found")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveReviewReq {
    pub review_id: i64,
    pub comment: Option<String>,
}

async fn review_approve(
    claims: Claims,
    state: State<AppState>,
    req: Json<ApproveReviewReq>,
) -> WebResult<()> {
    ensure_contributor(state.clone().0, claims.uid()).await?;

    if let Some(ref x) = req.comment && x.chars().count() > 1000 {
        err!("comment_too_long", "Comment is too long")
    }

    let mut review = SongPublishingReviewDao::get_by_id(&state.sql_pool, req.review_id).await?
        .ok_or_else(|| common!("not_found", "Review not found"))?;
    if review.status != 0 {
        err!("invalid_status", "Invalid review status")
    }
    
    let data: InternalSongPublishReviewData = serde_json::from_value(review.data.clone())
        .with_context(|| format!("Error during decoding song publish review({}) data", review.id))?;
    let uploader = UserDao::get_by_id(&state.sql_pool, review.user_id).await?
        .with_context(|| format!("User {} not found", review.user_id))?;

    let mut tx = state.sql_pool.begin().await?;

    // Formally insert data to the song table
    let song_id = SongDao::insert(&mut *tx, &data.song_info).await?;
    SongDao::update_song_origin_info(&mut tx, song_id, &data.song_origin_infos).await?;
    SongDao::update_song_production_crew(&mut tx, song_id, &data.song_production_crew).await?;
    // TODO: SongDao::update_song_external_links()

    let tag_ids = data.song_tags.iter().map(|x| x.id).collect();
    SongDao::update_song_tags(&mut tx, song_id, tag_ids).await?;

    // Update review data
    review.review_comment = req.comment.clone();
    review.review_time = Some(Utc::now());
    review.status = 1;
    SongPublishingReviewDao::update_by_id(&state.sql_pool, &review).await?;
    tx.commit().await?;

    // Write behind, data consistence is not guaranteed.
    search::song::add_song_document(
        state.meilisearch.as_ref(),
        song_id,
        &data.song_info,
        &data.song_production_crew,
        &data.song_origin_infos,
        &data.song_tags,
    ).await?;
    service::recommend_v2::notify_update(song_id, &state.redis_conn).await?;

    let email_cfg: EmailConfig = state.config.get_and_parse("email")?;
    service::mailer::send_review_approved_notification(
        &email_cfg,
        &uploader.email,
        &data.song_info.display_id,
        &data.song_info.title,
        &uploader.username,
        review.review_comment.as_deref()
    ).await?;
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectReviewReq {
    pub review_id: i64,
    pub comment: String,
}

async fn review_reject(
    claims: Claims,
    state: State<AppState>,
    req: Json<RejectReviewReq>,
) -> WebResult<()> {
    ensure_contributor(state.0.clone(), claims.uid()).await?;

    if req.comment.is_blank() {
        err!("comment_required", "Comment is required")
    }
    if req.comment.chars().count() > 1000 {
        err!("comment_too_long", "Comment is too long")
    }

    let mut review = SongPublishingReviewDao::get_by_id(&state.sql_pool, req.review_id).await?
        .ok_or_else(|| common!("not_found", "Review not found"))?;

    if review.status != 0 {
        err!("invalid_status", "Invalid review status")
    }
    let uploader = UserDao::get_by_id(&state.sql_pool, review.user_id).await?
        .with_context(|| format!("User {} not found", review.user_id))?;
    let data: InternalSongPublishReviewData = serde_json::from_value(review.data.clone())
        .with_context(|| format!("Error during decoding song publish review({}) data", review.id))?;

    review.review_comment = Some(req.comment.clone());
    review.review_time = Some(Utc::now());
    review.status = 2;
    SongPublishingReviewDao::update_by_id(&state.sql_pool, &review).await?;

    let email_cfg: EmailConfig = state.config.get_and_parse("email")?;
    service::mailer::send_review_approved_notification(
        &email_cfg,
        &uploader.email,
        &data.song_info.title,
        &data.song_info.display_id,
        &uploader.username,
        review.review_comment.as_deref()
    ).await?;
    ok!(())
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
