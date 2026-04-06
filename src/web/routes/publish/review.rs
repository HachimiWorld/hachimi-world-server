use crate::config::Config;
use crate::db::creator::CreatorDao;
use crate::db::song::{Song, SongDao, SongProductionCrew};
use crate::db::song_publishing_review::{ISongPublishingReviewDao, SongPublishingReview, SongPublishingReviewDao};
use crate::db::song_publishing_review_comment::{ISongPublishingReviewCommentDao, SongPublishingReviewComment, SongPublishingReviewCommentDao};
use crate::db::song_publishing_review_history::{ISongPublishingReviewHistoryDao, SongPublishingReviewHistory, SongPublishingReviewHistoryDao};
use crate::db::user::UserDao;
use crate::db::{song_publishing_review, song_publishing_review_history, CrudDao};
use crate::service::contributor::{check_contributor, ensure_contributor, CommunityCfg};
use crate::service::mailer::EmailConfig;
use crate::service::song::{CreationTypeInfo, ExternalLink};
use crate::service::{mailer, user};
use crate::util::IsBlank;
use crate::web::jwt::Claims;
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::routes::publish::{build_image_temp_key, build_internal_review_data, build_temp_key, parse_jmid, CreationInfo, InternalSongPublishReviewData, PageReq, PageResp, ProductionItem, SongPublishReviewBrief, SongTempData};
use crate::web::routes::song::TagItem;
use crate::web::routes::user::PublicUserProfile;
use crate::web::state::AppState;
use crate::{common, err, ok, search, service};
use anyhow::Context;
use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use redis::AsyncTypedCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use tracing::warn;

pub async fn page(
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
                    r#type: x.r#type,
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

pub async fn page_contributor(
    claims: Claims,
    state: State<AppState>,
    req: Query<PageReq>,
) -> WebResult<PageResp> {
    if req.page_size > 50 {
        err!("page_size_exceeded", "Page size too large");
    }
    ensure_contributor(&state, claims.uid()).await?;

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
                    r#type: x.r#type,
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
    pub comment: Option<String>,
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
    pub external_link: Vec<ExternalLink>,
    pub explicit: Option<bool>,
}

struct PublishSongPublishReviewMeta {
    review_id: i64,
    submit_time: DateTime<Utc>,
    review_time: Option<DateTime<Utc>>,
    comment: Option<String>,
    review_comment: Option<String>,
    status: i32,
}

async fn compose_publish_song_publish_review_data(
    sql_pool: &PgPool,
    meta: PublishSongPublishReviewMeta,
    data: InternalSongPublishReviewData,
) -> anyhow::Result<PublishSongPublishReviewData> {
    let uploader_name = UserDao::get_by_id(sql_pool, data.song_info.uploader_uid).await?
        .map(|x| x.username)
        .unwrap_or_else(|| {
            warn!("User {} not found during compose review({}) detail data", data.song_info.uploader_uid, meta.review_id);
            "Invalid".to_string()
        });

    let mut id_display_map = HashMap::new();
    for x in &data.song_origin_infos {
        match SongDao::get_by_id(sql_pool, x.song_id).await? {
            Some(y) => {
                id_display_map.insert(x.id, y.display_id);
            }
            None => {
                id_display_map.insert(x.id, "deleted".to_string());
            }
        }
    }

    let origin_infos_mapped = data.song_origin_infos.into_iter().map(|x| {
        let id = x.origin_song_id;
        CreationTypeInfo::from_song_origin_info(x, id.and_then(|x| id_display_map.get(&x).cloned()))
    }).collect();

    Ok(PublishSongPublishReviewData {
        review_id: meta.review_id,
        submit_time: meta.submit_time,
        review_time: meta.review_time,
        comment: meta.comment,
        review_comment: meta.review_comment,
        status: meta.status,
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
        uploader_name,
        external_link: data.song_external_links.into_iter().map(|x| ExternalLink {
            platform: x.platform,
            url: x.url,
        }).collect(),
        explicit: data.song_info.explicit,
    })
}

/// Get the review detail
///
/// Permission: Only available for the uploader and contributors.
pub async fn detail(
    claims: Claims,
    state: State<AppState>,
    req: Query<DetailReq>,
) -> WebResult<DetailResp> {
    let review = SongPublishingReviewDao::get_by_id(&state.sql_pool, req.review_id).await?;
    if let Some(review) = review {
        ensure_review_visible(&state, &review, claims.uid()).await?;

        let data = serde_json::from_value::<InternalSongPublishReviewData>(review.data)
            .with_context(|| format!("Error during decoding song publish review({}) data", review.id))?;

        let result = compose_publish_song_publish_review_data(
            &state.sql_pool,
            PublishSongPublishReviewMeta {
                review_id: review.id,
                submit_time: review.submit_time,
                review_time: review.review_time,
                comment: review.comment,
                review_comment: review.review_comment,
                status: review.status,
            },
            data,
        ).await?;
        ok!(result)
    } else {
        err!("not_found", "Review not found")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewModifyReq {
    pub review_id: i64,
    pub song_temp_id: Option<String>,
    pub cover_temp_id: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub lyrics: String,
    pub tag_ids: Vec<i64>,
    pub creation_info: CreationInfo,
    pub production_crew: Vec<ProductionItem>,
    pub external_links: Vec<ExternalLink>,
    pub explicit: bool,
    pub comment: Option<String>,
}

pub async fn review_modify(
    claims: Claims,
    mut state: State<AppState>,
    req: Json<ReviewModifyReq>,
) -> WebResult<()> {
    let _guard = state.red_lock.try_lock(&format!("lock:song_publish:{}", claims.uid())).await?
        .ok_or_else(|| common!("operation_in_progress", "Operation in progress"))?;

    if let Some(ref x) = req.comment && x.chars().count() > 1000 {
        err!("comment_too_long", "Comment is too long")
    }

    let mut review = SongPublishingReviewDao::get_by_id(&state.sql_pool, req.review_id).await?
        .ok_or_else(|| common!("not_found", "Review not found"))?;
    if review.user_id != claims.uid() {
        err!("permission_denied", "You are not allowed to modify this review")
    }
    if review.status != song_publishing_review::STATUS_PENDING {
        err!("invalid_status", "Invalid review status")
    }

    let current_data: InternalSongPublishReviewData = serde_json::from_value(review.data.clone())
        .with_context(|| format!("Error during decoding song publish review({}) data", review.id))?;

    let (file_url, duration_secs, gain) = if let Some(ref temp_id) = req.song_temp_id {
        let song_temp_data: Option<String> = state.redis_conn.get(build_temp_key(temp_id)).await?;
        let song_temp_data = song_temp_data
            .ok_or_else(|| common!("invalid_song_temp_id", "Invalid song temp id"))?;
        let song_temp_data: SongTempData = serde_json::from_str(&song_temp_data)?;
        (
            song_temp_data.file_url,
            song_temp_data.duration_secs,
            song_temp_data.gain,
        )
    } else {
        (
            current_data.song_info.file_url.clone(),
            current_data.song_info.duration_seconds as u64,
            current_data.song_info.gain,
        )
    };

    let cover_art_url = if let Some(ref temp_id) = req.cover_temp_id {
        let cover_url: Option<String> = state.redis_conn.get(build_image_temp_key(temp_id)).await?;
        cover_url
            .ok_or_else(|| common!("invalid_cover_temp_id", "Invalid cover temp id"))?
    } else {
        current_data.song_info.cover_art_url.clone()
    };

    let now = Utc::now();
    let song = Song {
        id: current_data.song_info.id,
        display_id: current_data.song_info.display_id.clone(),
        title: req.title.clone(),
        subtitle: req.subtitle.clone(),
        description: req.description.clone(),
        artist: current_data.song_info.artist.clone(),
        file_url,
        cover_art_url,
        lyrics: req.lyrics.clone(),
        duration_seconds: duration_secs as i32,
        uploader_uid: current_data.song_info.uploader_uid,
        creation_type: req.creation_info.creation_type,
        play_count: current_data.song_info.play_count,
        like_count: current_data.song_info.like_count,
        is_private: current_data.song_info.is_private,
        release_time: current_data.song_info.release_time,
        create_time: current_data.song_info.create_time,
        update_time: now,
        gain,
        explicit: Some(req.explicit),
    };

    let review_data = build_internal_review_data(
        &state.sql_pool,
        song,
        &req.tag_ids,
        &req.creation_info,
        &req.production_crew,
        &req.external_links,
    ).await?;

    let mut tx = state.sql_pool.begin().await?;
    let history_count = SongPublishingReviewHistoryDao::count_by_review_id(&mut *tx, review.id).await?;
    if history_count == 0 {
        SongPublishingReviewHistoryDao::insert(&mut *tx, &SongPublishingReviewHistory {
            id: 0,
            review_id: review.id,
            user_id: review.user_id,
            action_type: song_publishing_review_history::ACTION_SUBMIT,
            note: review.comment.clone(),
            snapshot_data: review.data.clone(),
            create_time: review.submit_time,
        }).await?;
    }

    review.data = serde_json::to_value(&review_data)?;
    review.update_time = now;
    review.comment = req.comment.clone();
    SongPublishingReviewDao::update_by_id(&mut *tx, &review).await?;
    SongPublishingReviewHistoryDao::insert(&mut *tx, &SongPublishingReviewHistory {
        id: 0,
        review_id: review.id,
        user_id: review.user_id,
        action_type: song_publishing_review_history::ACTION_MODIFY,
        note: req.comment.clone(),
        snapshot_data: serde_json::to_value(&review_data)?,
        create_time: now,
    }).await?;
    tx.commit().await?;

    let config = state.config.clone();
    let sql_pool = state.sql_pool.clone();
    let actor_uid = claims.uid();
    let review_id = review.id;
    let note = req.comment.clone();
    tokio::spawn(async move {
        send_review_modified_notification(&config, &sql_pool, review_id, actor_uid, note.as_deref()).await?;
        Ok::<(), anyhow::Error>(())
    });

    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCommentCreateReq {
    pub review_id: i64,
    pub content: String,
}

pub async fn review_comment_create(
    claims: Claims,
    state: State<AppState>,
    req: Json<ReviewCommentCreateReq>,
) -> WebResult<()> {
    if req.content.is_blank() {
        err!("comment_required", "Comment is required")
    }
    if req.content.chars().count() > 1000 {
        err!("comment_too_long", "Comment is too long")
    }

    let review = SongPublishingReviewDao::get_by_id(&state.sql_pool, req.review_id).await?
        .ok_or_else(|| common!("not_found", "Review not found"))?;
    if review.status != song_publishing_review::STATUS_PENDING {
        err!("review_closed", "The review is already closed")
    }

    ensure_review_comment_create_permission(&state, &review, claims.uid()).await?;

    let now = Utc::now();
    let comment = SongPublishingReviewComment {
        id: 0,
        review_id: review.id,
        user_id: claims.uid(),
        content: req.content.clone(),
        create_time: now,
        update_time: now,
    };
    SongPublishingReviewCommentDao::insert(&state.sql_pool, &comment).await?;

    let actor = UserDao::get_by_id(&state.sql_pool, claims.uid()).await?
        .ok_or_else(|| common!("user_not_found", "User not found"))?;
    let config = state.config.clone();
    let sql_pool = state.sql_pool.clone();
    let review_id = review.id;
    let actor_uid = actor.id;
    let actor_name = actor.username;
    let content = req.content.clone();
    tokio::spawn(async move {
        send_review_comment_notification(&config, &sql_pool, review_id, actor_uid, &actor_name, &content).await?;
        Ok::<(), anyhow::Error>(())
    });

    ok!(())
}



#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCommentListReq {
    pub review_id: i64,
    pub page_index: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCommentListResp {
    pub data: Vec<ReviewCommentItem>,
    pub page_index: i64,
    pub page_size: i64,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCommentItem {
    pub id: i64,
    pub review_id: i64,
    pub author: Option<PublicUserProfile>,
    pub content: String,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub async fn review_comment_list(
    claims: Claims,
    state: State<AppState>,
    req: Query<ReviewCommentListReq>,
) -> WebResult<ReviewCommentListResp> {
    if req.page_size > 50 {
        err!("page_size_exceeded", "Page size too large")
    }

    let review = SongPublishingReviewDao::get_by_id(&state.sql_pool, req.review_id).await?
        .ok_or_else(|| common!("not_found", "Review not found"))?;
    ensure_review_visible(&state, &review, claims.uid()).await?;

    let comments = SongPublishingReviewCommentDao::page_by_review_id(
        &state.sql_pool,
        req.review_id,
        req.page_index,
        req.page_size,
    ).await?;
    let total = SongPublishingReviewCommentDao::count_by_review_id(&state.sql_pool, req.review_id).await?;

    let mut data = Vec::with_capacity(comments.len());
    let uids = comments.iter().map(|x| x.user_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let users = user::get_public_profile(state.redis_conn.clone(), &state.sql_pool, &uids).await?;

    for comment in comments {
        data.push(ReviewCommentItem {
            id: comment.id,
            review_id: comment.review_id,
            author: users.get(&comment.user_id).cloned(),
            content: comment.content,
            create_time: comment.create_time,
            update_time: comment.update_time,
        });
    }

    ok!(ReviewCommentListResp {
        data,
        page_index: req.page_index,
        page_size: req.page_size,
        total,
    })
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCommentDeleteReq {
    pub comment_id: i64,
}

pub async fn review_comment_delete(
    claims: Claims,
    state: State<AppState>,
    req: Json<ReviewCommentDeleteReq>,
) -> WebResult<()> {
    let comment = SongPublishingReviewCommentDao::get_by_id(&state.sql_pool, req.comment_id).await?
        .ok_or_else(|| common!("not_found", "Comment not found"))?;
    let review = SongPublishingReviewDao::get_by_id(&state.sql_pool, comment.review_id).await?
        .ok_or_else(|| common!("not_found", "Review not found"))?;
    ensure_review_visible(&state, &review, claims.uid()).await?;
    ensure_review_comment_delete_permission(&state, &comment, claims.uid()).await?;
    SongPublishingReviewCommentDao::delete_by_id(&state.sql_pool, req.comment_id).await?;
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewHistoryListReq {
    pub review_id: i64,
    pub page_index: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewHistoryListResp {
    pub data: Vec<ReviewHistoryItem>,
    pub page_index: i64,
    pub page_size: i64,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewHistoryItem {
    pub id: i64,
    pub review_id: i64,
    pub action_type: i32,
    pub note: Option<String>,
    pub author: Option<PublicUserProfile>,
    pub create_time: DateTime<Utc>,
    pub snapshot: Option<PublishSongPublishReviewData>,
}

pub async fn review_history_list(
    claims: Claims,
    state: State<AppState>,
    req: Query<ReviewHistoryListReq>,
) -> WebResult<ReviewHistoryListResp> {
    if req.page_size > 50 {
        err!("page_size_exceeded", "Page size too large")
    }

    let review = SongPublishingReviewDao::get_by_id(&state.sql_pool, req.review_id).await?
        .ok_or_else(|| common!("not_found", "Review not found"))?;
    ensure_review_visible(&state, &review, claims.uid()).await?;

    let histories = SongPublishingReviewHistoryDao::page_by_review_id(
        &state.sql_pool,
        req.review_id,
        req.page_index,
        req.page_size,
    ).await?;
    let total = SongPublishingReviewHistoryDao::count_by_review_id(&state.sql_pool, req.review_id).await?;

    let uids = histories.iter().map(|x| x.user_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let users = user::get_public_profile(state.redis_conn.clone(), &state.sql_pool, &uids).await?;

    let mut data = Vec::with_capacity(histories.len());
    for x in histories {
        let snapshot = match serde_json::from_value::<InternalSongPublishReviewData>(x.snapshot_data) {
            Ok(v) => match compose_publish_song_publish_review_data(
                &state.sql_pool,
                PublishSongPublishReviewMeta {
                    review_id: x.review_id,
                    submit_time: x.create_time,
                    review_time: None,
                    comment: x.note.clone(),
                    review_comment: None,
                    status: song_publishing_review::STATUS_PENDING,
                },
                v,
            ).await {
                Ok(v) => Some(v),
                Err(err) => {
                    warn!("Error during composing song publish review history({}) data: {:?}", x.id, err);
                    None
                }
            },
            Err(err) => {
                warn!("Error during decoding song publish review history({}) data: {:?}", x.id, err);
                None
            }
        };

        data.push(ReviewHistoryItem {
            id: x.id,
            review_id: x.review_id,
            action_type: x.action_type,
            note: x.note,
            author: users.get(&x.user_id).cloned(),
            create_time: x.create_time,
            snapshot,
        });
    }

    ok!(ReviewHistoryListResp {
        data,
        page_index: req.page_index,
        page_size: req.page_size,
        total,
    })
}

async fn ensure_review_visible(
    state: &AppState,
    review: &SongPublishingReview,
    uid: i64,
) -> Result<(), WebError<CommonError>> {
    if review.user_id == uid {
        return Ok(());
    }

    let is_contributor = check_contributor(
        &state.config,
        state.redis_conn.clone(),
        &state.red_lock,
        &state.sql_pool,
        uid,
    ).await?;
    if is_contributor {
        return Ok(());
    }

    err!("permission_denied", "You are not allowed to view this review")
}

async fn ensure_review_comment_create_permission(
    state: &AppState,
    review: &SongPublishingReview,
    uid: i64,
) -> Result<(), WebError<CommonError>> {
    if review.user_id == uid {
        return Ok(());
    }

    let is_contributor = check_contributor(
        &state.config,
        state.redis_conn.clone(),
        &state.red_lock,
        &state.sql_pool,
        uid,
    ).await?;
    if !is_contributor {
        err!("permission_denied", "Only the uploader or a contributor can comment")
    }
    Ok(())
}

async fn ensure_review_comment_delete_permission(
    state: &AppState,
    comment: &SongPublishingReviewComment,
    uid: i64,
) -> Result<(), WebError<CommonError>> {
    if comment.user_id == uid {
        return Ok(());
    }

    let is_contributor = check_contributor(
        &state.config,
        state.redis_conn.clone(),
        &state.red_lock,
        &state.sql_pool,
        uid,
    ).await?;
    if !is_contributor {
        err!("permission_denied", "Only the comment author or a contributor can delete comments")
    }
    Ok(())
}

async fn send_review_comment_notification(
    config: &Config,
    sql_pool: &PgPool,
    review_id: i64,
    actor_uid: i64,
    actor_name: &str,
    content: &str,
) -> anyhow::Result<()> {
    let email_cfg: EmailConfig = config.get_and_parse("email")?;
    let community_cfg: CommunityCfg = config.get_and_parse("community")?;
    let review = SongPublishingReviewDao::get_by_id(sql_pool, review_id).await?
        .with_context(|| format!("Review {} not found", review_id))?;
    let actor = UserDao::get_by_id(sql_pool, actor_uid).await?
        .with_context(|| format!("User {} not found", actor_uid))?;
    let uploader = UserDao::get_by_id(sql_pool, review.user_id).await?
        .with_context(|| format!("User {} not found", review.user_id))?;

    let mut recipients = HashSet::new();
    recipients.insert(uploader.email.clone());
    recipients.extend(community_cfg.contributors);
    recipients.remove(&actor.email);

    let subject = format!("稿件评论更新：{}", review.song_display_id);
    let body = format!(
        "{actor_name} 在稿件 {} 下发表了新评论：\n\n{}",
        review.song_display_id,
        content,
    );

    for email in recipients {
        mailer::send_notification(&email_cfg, &email, &subject, &body).await?;
    }
    Ok(())
}

async fn send_review_modified_notification(
    config: &Config,
    sql_pool: &PgPool,
    review_id: i64,
    actor_uid: i64,
    note: Option<&str>,
) -> anyhow::Result<()> {
    let email_cfg: EmailConfig = config.get_and_parse("email")?;
    let community_cfg: CommunityCfg = config.get_and_parse("community")?;
    let review = SongPublishingReviewDao::get_by_id(sql_pool, review_id).await?
        .with_context(|| format!("Review {} not found", review_id))?;
    let actor = UserDao::get_by_id(sql_pool, actor_uid).await?
        .with_context(|| format!("User {} not found", actor_uid))?;

    let mut recipients = HashSet::new();
    recipients.extend(community_cfg.contributors);
    recipients.remove(&actor.email);

    let subject = format!("稿件已更新：{}", review.song_display_id);
    let body = format!(
        "{} 更新了稿件 {}。{}",
        actor.username,
        review.song_display_id,
        note.map(|x| format!("\n\n备注：{x}")).unwrap_or_default(),
    );

    for email in recipients {
        mailer::send_notification(&email_cfg, &email, &subject, &body).await?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveReviewReq {
    pub review_id: i64,
    pub comment: Option<String>,
}

pub async fn review_approve(
    claims: Claims,
    state: State<AppState>,
    req: Json<ApproveReviewReq>,
) -> WebResult<()> {
    ensure_contributor(&state, claims.uid()).await?;

    if let Some(ref x) = req.comment && x.chars().count() > 1000 {
        err!("comment_too_long", "Comment is too long")
    }

    let mut review = SongPublishingReviewDao::get_by_id(&state.sql_pool, req.review_id).await?
        .ok_or_else(|| common!("not_found", "Review not found"))?;
    if review.status != 0 {
        err!("invalid_status", "Invalid review status")
    }

    let mut data: InternalSongPublishReviewData = serde_json::from_value(review.data.clone())
        .with_context(|| format!("Error during decoding song publish review({}) data", review.id))?;
    let uploader = UserDao::get_by_id(&state.sql_pool, review.user_id).await?
        .with_context(|| format!("User {} not found", review.user_id))?;

    let mut tx = state.sql_pool.begin().await?;

    // Update review data
    review.review_comment = req.comment.clone();
    review.review_time = Some(Utc::now());
    review.status = 1;
    SongPublishingReviewDao::update_by_id(&mut *tx, &review).await?;

    if review.r#type == song_publishing_review::TYPE_CREATE {
        // Create new song

        // Formally insert data to the song table
        data.song_info.create_time = Utc::now();
        let song_id = SongDao::insert(&mut *tx, &data.song_info).await?;

        // Update corresponding data
        let tag_ids = data.song_tags.iter().map(|x| x.id).collect();
        SongDao::update_song_origin_info(&mut tx, song_id, &data.song_origin_infos).await?;
        SongDao::update_song_production_crew(&mut tx, song_id, &data.song_production_crew).await?;
        SongDao::update_song_external_links(&mut tx, song_id, &data.song_external_links).await?;
        SongDao::update_song_tags(&mut tx, song_id, tag_ids).await?;

        // Activate the jmid prefix
        let (prefix, _number) = parse_jmid(&review.song_display_id)
            .ok_or_else(|| common!("invalid_song_display_id", "Invalid song display id"))?; // This error should never happen

        let creator = CreatorDao::get_by_user_id(&mut *tx, review.user_id).await?;
        if let Some(mut x) = creator
            && x.jmid_prefix == prefix && x.active == false
        {
            // This is a first PR, and we should activate the jmid prefix
            x.active = true;
            x.update_time = Utc::now();
            CreatorDao::update_by_id(&mut *tx, &x).await?;
        } else {
            // This pr might be the old data, do not create creator, just ignore
        }
        tx.commit().await?;

        // Write behind, data consistence is not guaranteed.
        search::song::add_or_replace_document(
            &state.meilisearch,
            &state.sql_pool,
            &[song_id],
        ).await?;
        service::recommend_v2::notify_update(song_id, state.redis_conn.clone()).await?;

        let email_cfg: EmailConfig = state.config.get_and_parse("email")?;
        service::mailer::send_review_approved_notification(
            &email_cfg,
            &uploader.email,
            &data.song_info.display_id,
            &data.song_info.title,
            &uploader.username,
            review.review_comment.as_deref(),
        ).await?;
    } else if review.r#type == song_publishing_review::TYPE_MODIFY {
        // Update existing song
        let song_id = data.song_info.id;
        let orig_song = SongDao::get_by_id(&mut *tx, song_id).await?
            .ok_or_else(|| common!("not_found", "Song not found"))?;
        let new_song = Song {
            id: orig_song.id,
            display_id: orig_song.display_id,
            title: data.song_info.title,
            subtitle: data.song_info.subtitle,
            description: data.song_info.description,
            artist: data.song_info.artist,
            file_url: data.song_info.file_url,
            cover_art_url: data.song_info.cover_art_url,
            lyrics: data.song_info.lyrics,
            duration_seconds: data.song_info.duration_seconds,
            uploader_uid: data.song_info.uploader_uid,
            creation_type: data.song_info.creation_type,
            play_count: data.song_info.play_count,
            like_count: data.song_info.like_count,
            is_private: data.song_info.is_private,
            release_time: data.song_info.release_time,
            create_time: orig_song.create_time,
            update_time: Utc::now(), // Current time
            explicit: data.song_info.explicit,
            gain: data.song_info.gain,
        };

        SongDao::update_by_id(&mut *tx, &new_song).await?;
        // Update corresponding data
        let tag_ids = data.song_tags.iter().map(|x| x.id).collect();
        SongDao::update_song_origin_info(&mut tx, song_id, &data.song_origin_infos).await?;
        SongDao::update_song_production_crew(&mut tx, song_id, &data.song_production_crew).await?;
        SongDao::update_song_external_links(&mut tx, song_id, &data.song_external_links).await?;
        SongDao::update_song_tags(&mut tx, song_id, tag_ids).await?;
        tx.commit().await?;

        search::song::add_or_replace_document(
            &state.meilisearch,
            &state.sql_pool,
            &[song_id],
        ).await?;
        service::recommend_v2::notify_update(song_id, state.redis_conn.clone()).await?;

        let email_cfg: EmailConfig = state.config.get_and_parse("email")?;
        service::mailer::send_review_modify_approved_notification(
            &email_cfg,
            &uploader.email,
            &data.song_info.display_id,
            &uploader.username,
            review.review_comment.as_deref(),
        ).await?;
    }
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectReviewReq {
    pub review_id: i64,
    pub comment: String,
}

pub async fn review_reject(
    claims: Claims,
    state: State<AppState>,
    req: Json<RejectReviewReq>,
) -> WebResult<()> {
    ensure_contributor(&state, claims.uid()).await?;

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

    review.review_comment = Some(req.comment.clone());
    review.review_time = Some(Utc::now());
    review.status = 2;

    let mut tx = state.sql_pool.begin().await?;
    SongPublishingReviewDao::update_by_id(&mut *tx, &review).await?;

    if review.r#type == song_publishing_review::TYPE_CREATE {
        let data: InternalSongPublishReviewData = serde_json::from_value(review.data.clone())
            .with_context(|| format!("Error during decoding song publish review({}) data", review.id))?;

        let (prefix, _) = parse_jmid(&review.song_display_id).ok_or_else(|| common!("invalid_jmid", "Invalid jmid"))?;

        let creator = CreatorDao::get_by_user_id(&mut *tx, review.user_id).await?;
        if let Some(x) = creator
            && x.jmid_prefix == prefix && x.active == false
        {
            // This is a first PR.
            // Delete the creator record to release the prefix lock
            CreatorDao::delete_by_id(&mut *tx, x.id).await?;
        } else {
            // This pr might be the old data, do not create creator, just ignore
        }
        tx.commit().await?;

        let email_cfg: EmailConfig = state.config.get_and_parse("email")?;
        service::mailer::send_review_rejected_notification(
            &email_cfg,
            &uploader.email,
            &review.song_display_id,
            &data.song_info.title,
            &uploader.username,
            &req.comment,
        ).await?;
    } else if review.r#type == song_publishing_review::TYPE_MODIFY {
        tx.commit().await?;

        let email_cfg: EmailConfig = state.config.get_and_parse("email")?;
        service::mailer::send_review_modify_rejected_notification(
            &email_cfg,
            &uploader.email,
            &review.song_display_id,
            &uploader.username,
            &req.comment,
        ).await?;
    }
    ok!(())
}
