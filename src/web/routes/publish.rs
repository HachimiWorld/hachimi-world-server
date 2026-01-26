use crate::audio::ParseError;
use crate::config::Config;
use crate::db::creator::{Creator, CreatorDao};
use crate::db::song::{ISongDao, Song, SongDao, SongExternalLink, SongOriginInfo, SongProductionCrew};
use crate::db::song_publishing_review::{ISongPublishingReviewDao, SongPublishingReview, SongPublishingReviewDao};
use crate::db::song_tag::{ISongTagDao, SongTag, SongTagDao};
use crate::db::user::UserDao;
use crate::db::{song_publishing_review, CrudDao};
use crate::service::contributor::ensure_contributor;
use crate::service::mailer;
use crate::service::mailer::EmailConfig;
use crate::service::song::{CreationTypeInfo, ExternalLink};
use crate::service::upload::{scale_down_to_webp, ResizeType};
use crate::util::{validate_platforms, IsBlank};
use crate::web::jwt::Claims;
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::routes::song::TagItem;
use crate::web::state::AppState;
use crate::{audio, common, err, ok, search, service};
use anyhow::Context;
use async_backtrace::framed;
use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use itertools::Itertools;
use redis::AsyncTypedCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use std::io::Cursor;
use tracing::{info, warn};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/upload_audio_file", post(upload_audio_file).layer(DefaultBodyLimit::max(20 * 1024 * 1024)) )
        .route("/upload_cover_image", post(upload_cover_image).layer(DefaultBodyLimit::max(10 * 1024 * 1024)) )
        .route("/publish", post(publish))
        .route("/modify", post(modify))
        .route("/delete", post(delete))
        .route("/change_jmid", post(change_jmid))
        .route("/review/page", get(page))
        .route("/review/page_contributor", get(page_contributor))
        .route("/review/detail", get(detail))
        .route("/review/approve", post(review_approve))
        .route("/review/reject", post(review_reject))
        // .route("/review/modify", post(review_modify))
        // .route("/review/comment/create", post(review_comment))
        // .route("/review/comment/list", post())
        // .route("/review/comment/delete", post())
        .route("/jmid/check_prefix", get(jmid_check_prefix))
        .route("/jmid/check", get(jmid_check))
        .route("/jmid/mine", get(jmid_mine))
        .route("/jmid/get_next", get(jmid_get_next))
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishReq {
    pub song_temp_id: String,
    pub cover_temp_id: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub lyrics: String,
    pub tag_ids: Vec<i64>,
    pub creation_info: CreationInfo,
    pub production_crew: Vec<ProductionItem>,
    pub external_links: Vec<ExternalLink>,
    /// @since 251105, should be required in new client.
    pub explicit: Option<bool>,
    /// @since 251114, should be required in new client.
    pub jmid: Option<String>,
    /// @since 251114
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreationInfo {
    /// 0: original, 1: derivative work, 2: tertiary work
    pub creation_type: i32,
    // TODO: A derivation song can be inspired by many origin songs
    pub origin_info: Option<CreationTypeInfo>,
    pub derivative_info: Option<CreationTypeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductionItem {
    pub role: String,
    pub uid: Option<i64>,
    pub name: Option<String>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResp {
    pub review_id: i64,
    pub song_display_id: String,
}

#[framed]
pub async fn publish(
    claims: Claims,
    mut state: State<AppState>,
    mut req: Json<PublishReq>,
) -> WebResult<PublishResp> {
    let _guard = state.red_lock.try_lock(&format!("lock:song_publish:{}", claims.uid())).await?
        .ok_or_else(|| common!("operation_in_progress", "Operation in progress"))?;

    let uid = claims.uid();
    let user = UserDao::get_by_id(&state.sql_pool, uid).await?.ok_or_else(|| common!("user_not_found", "User not found"))?;

    // Processing data
    let song_temp_data: Option<String> = state.redis_conn.get(build_temp_key(&req.song_temp_id)).await?;
    let song_temp_data = song_temp_data.ok_or_else(|| common!("invalid_song_temp_id", "Invalid song temp id"))?;
    let song_temp_data: SongTempData = serde_json::from_str(&song_temp_data)?;

    let cover_url: Option<String> = state.redis_conn.get(build_image_temp_key(&req.cover_temp_id)).await?;
    let cover_url = cover_url.ok_or_else(|| common!("invalid_cover_temp_id", "Invalid cover temp id"))?;

    // Check the jmid
    let create_new_jmid: bool;
    let jmid = match &req.jmid {
        Some(x) => {
            // The creator provided a jmid. We check whether the jmid is available
            // Firstly, check the prefix
            let (prefix, _) = parse_jmid(&x).ok_or_else(|| common!("invalid_jmid", "Invalid jmid"))?;
            create_new_jmid = check_jmid_prefix_for_publication(&state.sql_pool, claims.uid(), prefix).await?;

            // Secondly, check the full jmid
            let available = check_jmid_available(&state.sql_pool, &x).await?;
            if !available {
                err!("jmid_already_used", "The jmid ({}) is already used", x)
            }

            // Take the user provided jmid
            x.clone()
        }
        None => {
            // For backward compatibility. If the creator doesn't provide a jmid, we use the random jm-id, and do not insert to creators table
            create_new_jmid = false;
            loop {
                let id = service::song::generate_song_display_id();
                if SongDao::get_by_display_id(&state.sql_pool, &id).await?.is_some() {
                    continue;
                }
                break id;
            }
        }
    };


    let now = Utc::now();

    let song = Song {
        id: 0,
        display_id: jmid.to_string(),
        title: req.title.to_string(),
        subtitle: req.subtitle.to_string(),
        description: req.description.to_string(),
        artist: user.username.to_string(),
        file_url: song_temp_data.file_url.to_string(),
        cover_art_url: cover_url.to_string(),
        lyrics: req.lyrics.to_string(),
        duration_seconds: song_temp_data.duration_secs as i32, // Fuck the num type
        uploader_uid: user.id,
        creation_type: req.creation_info.creation_type,
        play_count: 0,
        like_count: 0,
        is_private: false,
        release_time: now,
        create_time: now,
        update_time: now, // Do we really need three time data?
        gain: song_temp_data.gain,
        explicit: req.explicit,
    };

    let review_data = build_internal_review_data(
        &state.sql_pool,
        song,
        &req.tag_ids,
        &req.creation_info,
        &req.production_crew,
        &req.external_links,
    ).await?;

    let review = SongPublishingReview {
        id: 0,
        user_id: claims.uid(),
        song_display_id: jmid.clone(),
        data: serde_json::to_value(review_data)?,
        submit_time: now,
        update_time: now,
        review_time: None,
        review_comment: None,
        status: song_publishing_review::STATUS_PENDING,
        r#type: song_publishing_review::TYPE_CREATE,
        comment: req.comment.take(),
    };

    let mut tx = state.sql_pool.begin().await?;
    let review_id = SongPublishingReviewDao::insert(&mut *tx, &review).await?;

    if create_new_jmid {
        let (jmid_prefix, _) = parse_jmid(&jmid).ok_or_else(|| common!("invalid_jmid", "Invalid jmid"))?;
        let _guard = state.red_lock.try_lock(&format!("lock:jmid_creation:{}", jmid_prefix)).await?
            .ok_or_else(|| common!("operation_in_progress", "Operation in progress"))?;

        // Lock the jmid by creating a creator record with `active=false`
        CreatorDao::insert(&mut *tx, &Creator {
            id: 0,
            user_id: claims.uid(),
            jmid_prefix: jmid_prefix.into(),
            active: false,
            create_time: now,
            update_time: now,
        }).await?;
    }
    tx.commit().await?;

    // TODO: Refactor with message queue
    tokio::spawn(async move {
        send_notification_to_maintainer(&state.config, &req.title, &user.username).await?;
        Ok::<(), anyhow::Error>(())
    });

    ok!(PublishResp {
        review_id: review_id,
        song_display_id: jmid
    })
}

async fn send_notification_to_maintainer(
    config: &Config,
    title: &str,
    author: &str
) -> anyhow::Result<()> {
    let email_cfg: EmailConfig = config.get_and_parse("email")?;
    let community_cfg: CommunityCfg = config.get_and_parse("community")?;
    if let Some(email) = community_cfg.contributors.first() {
        mailer::send_notification(&email_cfg, email, "有新的稿件待审核", &format!("{} - {}", title, author)).await?;
    }
    Ok(())
}

async fn build_internal_review_data(
    sql_pool: &PgPool,
    mut song: Song,
    tag_ids: &[i64],
    creation_info: &CreationInfo,
    production_crew_req: &[ProductionItem],
    external_links_req: &[ExternalLink],
) -> Result<InternalSongPublishReviewData, WebError<CommonError>> {
    // Validate creation_type
    if creation_info.creation_type == 1 && creation_info.origin_info.is_none() {
        err!("missing_origin_info", "Missing origin info for derivative song");
    }
    if creation_info.creation_type == 2 && creation_info.derivative_info.is_none() {
        err!("missing_origin_info", "Missing derivative info for derivative song");
    }

    // Validate and load tags
    let tags = SongTagDao::list_by_ids(sql_pool, tag_ids).await?;
    if tags.len() != tag_ids.len() {
        err!("tag_not_found", "Some tags not found");
    }

    // Origin infos
    let mut song_origin_infos = Vec::new();
    for x in [
        &creation_info.origin_info,
        &creation_info.derivative_info,
    ] {
        if let Some(item) = x {
            // Validate, must set one of the id or title
            if item.song_display_id.is_none() && item.title.is_none() {
                err!("title_missed", "Origin info title must not be empty")
            }
            // Parse internal song id
            let song_ref = if let Some(ref display_id) = item.song_display_id {
                let song = SongDao::get_by_display_id(sql_pool, display_id)
                    .await?
                    .ok_or_else(|| {
                        common!(
                                "song_not_found",
                                "The song (ID={}) specified in origin info was not found",
                                display_id
                            )
                    })?;
                Some(song)
            } else {
                None
            };
            // Add to batch
            song_origin_infos.push(SongOriginInfo {
                id: 0,
                song_id: 0,
                origin_type: item.origin_type,
                origin_song_id: song_ref.map(|x| x.id),
                origin_title: item.title.clone(),
                origin_artist: item.artist.clone(),
                origin_url: item.url.clone(),
            });
        }
    }

    // Production crew
    let mut production_crew = Vec::new();
    for member in production_crew_req {
        if member.uid.is_none() && member.name.is_none() {
            err!("name_missed", "One of uid or name must be set")
        }

        if let Some(uid) = member.uid {
            let user = UserDao::get_by_id(sql_pool, uid).await?
                .ok_or_else(|| common!("crew_user_not_found", "Crew user not found"))?;
            production_crew.push(SongProductionCrew {
                id: 0,
                song_id: 0,
                role: member.role.clone(),
                uid: Some(user.id),
                person_name: Some(user.username),
            });
        }

        if let Some(ref name) = member.name {
            production_crew.push(SongProductionCrew {
                id: 0,
                song_id: 0,
                role: member.role.clone(),
                uid: None,
                person_name: Some(name.clone()),
            });
        }
    }

    // Update artist based on production crew
    song.artist = production_crew
        .iter()
        .map(|x| {
            x.person_name
                .clone()
                .unwrap_or_else(|| "Unknown".to_string())
        })
        .join(", ");

    // External links
    let mut links = Vec::new();
    for link in external_links_req {
        validate_platforms(&link.platform, &link.url)?;

        let x = SongExternalLink {
            id: 0,
            song_id: 0,
            platform: link.platform.clone(),
            url: link.url.clone(),
        };
        links.push(x)
    }

    Ok(InternalSongPublishReviewData {
        song_info: song,
        song_origin_infos,
        song_production_crew: production_crew,
        song_tags: tags,
        song_external_links: links,
    })
}

/// Check whether the `jmid_prefix` is available for publication. And returns whether the prefix is **new** if available. Error if the jmid is not available.
///
/// The `jmid_prefix` is unique for each creator. It could be locked by a user because we have an audition procedure.
///
/// Basically, we have these situations:
/// - `never_used`: We can lock (create a creator record with `active == false`) it for the first time.
/// - `owned`: We can just use it.
/// - `not_match`: The prefix does not match the creator's own prefix.
/// - `locked_by_self`: The first publishing request is in progress.
/// - `used`: Locked or owned by another user.
async fn check_jmid_prefix_for_publication(
    sql_pool: &PgPool,
    user_id: i64,
    jmid_prefix: &str
) -> Result<bool, WebError<CommonError>> {
    let create_new_jmid: bool;
    let creator = CreatorDao::get_by_user_id(sql_pool, user_id).await?;
    match creator {
        Some(x) => {
            // owned or locked_by_self
            if x.active {
                if x.jmid_prefix == jmid_prefix {
                    // owned
                    create_new_jmid = false;
                } else {
                    // not_match
                    err!("jmid_prefix_mismatch", "The jmid prefix ({}) does not match the your prefix ({})", jmid_prefix, x.jmid_prefix)
                }
            } else {
                // locked_by_self
                err!("pending", "Please wait for your first publishing to complete.")
            }
        }
        None => {
            // never_used or used
            let r = CreatorDao::get_by_jmid_prefix(sql_pool, jmid_prefix).await?;
            match r {
                Some(creator) => {
                    // used
                    err!("jmid_prefix_already_used", "The jmid prefix ({}) is already used by another user {}", jmid_prefix, creator.user_id)
                }
                None => {
                    info!("lock the new jmid {}", jmid_prefix);
                    // never_used
                    create_new_jmid = true;
                }
            }
        }
    }

    Ok(create_new_jmid)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModifyResp {
    pub review_id: i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModifyReq {
    pub song_id: i64,
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
    pub explicit: bool, // It's required because this is a new api
    pub comment: Option<String>,
}

pub async fn modify(
    claims: Claims,
    mut state: State<AppState>,
    mut req: Json<ModifyReq>,
) -> WebResult<ModifyResp> {
    let _guard = state.red_lock.try_lock(&format!("lock:song_publish:{}", claims.uid())).await?
        .ok_or_else(|| common!("operation_in_progress", "Operation in progress"))?;

    // Check ownership and load original song
    let orig_song = SongDao::get_by_id(&state.sql_pool, req.song_id)
        .await?
        .ok_or_else(|| common!("song_not_found", "Song was not found"))?;
    if orig_song.uploader_uid != claims.uid() {
        err!("permission_denied", "You are not allowed to modify this song");
    }

    let now = Utc::now();

    // Resolve audio (use temp if provided, otherwise original)
    let (file_url, duration_secs, gain) = if let Some(ref temp_id) = req.song_temp_id {
        let song_temp_data: Option<String> =
            state.redis_conn.get(build_temp_key(temp_id)).await?;
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
            orig_song.file_url.clone(),
            orig_song.duration_seconds as u64,
            orig_song.gain,
        )
    };

    // Resolve cover (use temp if provided, otherwise original)
    let cover_art_url = if let Some(ref temp_id) = req.cover_temp_id {
        let cover_url: Option<String> =
            state.redis_conn.get(build_image_temp_key(temp_id)).await?;
        cover_url
            .ok_or_else(|| common!("invalid_cover_temp_id", "Invalid cover temp id"))?
    } else {
        orig_song.cover_art_url.clone()
    };

    // Build song snapshot for review (based on original, but with new metadata)
    let song = Song {
        id: orig_song.id,
        display_id: orig_song.display_id.clone(),
        title: req.title.to_string(),
        subtitle: req.subtitle.to_string(),
        description: req.description.to_string(),
        // artist will be overwritten from production crew in helper
        artist: orig_song.artist.clone(),
        file_url,
        cover_art_url,
        lyrics: req.lyrics.to_string(),
        duration_seconds: duration_secs as i32,
        uploader_uid: orig_song.uploader_uid,
        creation_type: req.creation_info.creation_type,
        // keep stats and visibility from original
        play_count: orig_song.play_count,
        like_count: orig_song.like_count,
        is_private: orig_song.is_private,
        release_time: orig_song.release_time,
        create_time: orig_song.create_time,
        update_time: now,
        gain,
        // If explicit is provided, override; otherwise keep original
        explicit: Some(req.explicit),
    };

    // Reuse the same validation and data-building logic as `publish`
    let data = build_internal_review_data(
        &state.sql_pool,
        song,
        &req.tag_ids,
        &req.creation_info,
        &req.production_crew,
        &req.external_links,
    )
        .await?;

    // Create the modify SR
    let review = SongPublishingReview {
        id: 0,
        user_id: claims.uid(),
        song_display_id: orig_song.display_id.clone(),
        data: serde_json::to_value(data)?,
        submit_time: now,
        update_time: now,
        review_time: None,
        review_comment: None,
        status: song_publishing_review::STATUS_PENDING,
        // TYPE_MODIFY review
        r#type: song_publishing_review::TYPE_MODIFY,
        comment: req.comment.take(),
    };

    let review_id = SongPublishingReviewDao::insert(&state.sql_pool, &review).await?;

    ok!(ModifyResp { review_id: review_id })

}

pub async fn delete() -> WebResult<()> {
    err!("no_impl", "Not implemented yet");
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadAudioFileResp {
    pub temp_id: String,
    pub duration_secs: u64,
    pub title: Option<String>,
    pub bitrate: Option<String>,
    pub artist: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongTempData {
    pub file_url: String,
    pub duration_secs: u64,
    pub gain: Option<f32>,
}

#[framed]
pub async fn upload_audio_file(
    _claims: Claims,
    mut state: State<AppState>,
    mut multipart: Multipart,
) -> WebResult<UploadAudioFileResp> {
    // 1. Receive streams
    let data_field = multipart
        .next_field()
        .await?
        .with_context(|| "No data field found")?;

    let file_name = data_field.file_name().map(|x| x.to_string());

    // TODO[opt](song): decode and receive in parallel
    let bytes = data_field.bytes().await?;
    let cursor = Cursor::new(bytes.clone());

    // 2. Validate metadata
    let metadata =
        match audio::parse_and_validate(Box::new(cursor), file_name.as_ref().map(|x| x.as_str())) {
            Ok(v) => v,
            Err(err) => match err {
                ParseError::FormatUnsupported => {
                    err!("format_unsupported", "Audio format not supported")
                }
                ParseError::TrackNotFound => err!("track_not_found", "Audio track not found"),
                ParseError::MetadataNotFound(key) => err!(
                    "metadata_not_found",
                    "Metadata {key} not found in audio"
                ),
                ParseError::ParsingDurationError => err!("parsing_duration_error", "Failed to parse duration"),
                ParseError::Parse(err) => {
                    tracing::error!("Error parsing audio: {:?}", err);
                    err!("parse_error", "Error parsing audio")
                }
                ParseError::CalculatingGainPeakError => err!("calculating_gain_peak_error", "Failed to calculate gain and peak"),
            },
        };

    // 3. Upload to s3
    // Generate a random filename
    let file_name = format!("{}.{}", uuid::Uuid::new_v4(), metadata.format);
    let result = state
        .file_host
        .upload(bytes, &format!("songs/{}", file_name))
        .await?;

    let temp_id = uuid::Uuid::new_v4().to_string();
    let data = serde_json::to_string(&SongTempData {
        file_url: result.public_url.to_string(),
        duration_secs: metadata.duration_secs,
        gain: Some(metadata.gain_db),
    })?;
    let _: () = state
        .redis_conn
        .set_ex(build_temp_key(&temp_id), data, 3600)
        .await?;

    ok!(UploadAudioFileResp {
        temp_id: temp_id,
        title: metadata.title,
        duration_secs: metadata.duration_secs,
        bitrate: None,
        artist: None,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadImageResp {
    pub temp_id: String,
}

#[framed]
pub async fn upload_cover_image(
    claims: Claims,
    mut state: State<AppState>,
    mut multipart: Multipart,
) -> WebResult<UploadImageResp> {
    let _ = if let Some(x) = UserDao::get_by_id(&state.sql_pool, claims.uid()).await? {
        x
    } else {
        err!("not_found", "User not found")
    };

    let data_field = multipart
        .next_field()
        .await?
        .with_context(|| "No data field found")?;
    let bytes = data_field.bytes().await?;

    // Validate image
    if bytes.len() > 8 * 1024 * 1024 {
        err!("image_too_large", "Image size must be less than 8MB");
    }

    let webp = scale_down_to_webp(1024, 1024, bytes.clone(), ResizeType::Fit, 90f32)
        .map_err(|_| common!("invalid_image", "The image is not supported"))?;

    // Upload image
    let sha1 = openssl::sha::sha1(&webp);
    let filename = format!("images/cover/{}.webp", hex::encode(sha1));
    let result = state.file_host.upload(webp.into(), &filename).await?;
    let temp_id = uuid::Uuid::new_v4().to_string();
    let _: () = state.redis_conn
        .set_ex(build_image_temp_key(&temp_id), result.public_url, 3600)
        .await?;

    ok!(UploadImageResp { temp_id })
}

fn build_temp_key(temp_id: &str) -> String {
    let key = format!("song_upload:temp:{}", temp_id);
    key
}
fn build_image_temp_key(temp_id: &str) -> String {
    let key = format!("songs_upload:cover_temp:{}", temp_id);
    key
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
    /// @since 251117
    pub r#type: i32
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
                r#type: value.r#type,
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
    pub song_external_links: Vec<SongExternalLink>,
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

async fn page_contributor(
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

/// Get the review detail
///
/// Permission: Only available for the uploader and contributors.
async fn detail(
    claims: Claims,
    state: State<AppState>,
    req: Query<DetailReq>,
) -> WebResult<DetailResp> {
    let review = SongPublishingReviewDao::get_by_id(&state.sql_pool, req.review_id).await?;
    if let Some(review) = review {
        // Permission check
        if review.user_id != claims.uid() {
            ensure_contributor(&state, claims.uid()).await?;
        }

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
            external_link: data.song_external_links.into_iter().map(|x| ExternalLink {
                platform: x.platform,
                url: x.url,
            }).collect(),
            explicit: data.song_info.explicit,
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

async fn review_reject(
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidCheckPReq {
    pub jmid_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidCheckPResp {
    pub result: bool,
}

/// Check if the jmid prefix part is not used by anyone
async fn jmid_check_prefix(
    _: Claims,
    state: State<AppState>,
    req: Query<JmidCheckPReq>,
) -> WebResult<JmidCheckPResp> {
    let r = CreatorDao::get_by_jmid_prefix(&state.sql_pool, &req.jmid_prefix).await?;
    ok!(JmidCheckPResp {result: r.is_none()})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidCheckReq {
    pub jmid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidCheckResp {
    pub result: bool,
}

/// Check if the full jmid is available
async fn jmid_check(
    _: Claims,
    state: State<AppState>,
    req: Query<JmidCheckReq>,
) -> WebResult<JmidCheckResp> {
    let r = check_jmid_available(&state.sql_pool, &req.jmid).await?;
    ok!(JmidCheckResp {result: r})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidMineResp {
    pub jmid_prefix: Option<String>,
}

async fn jmid_mine(
    claims: Claims,
    state: State<AppState>,
) -> WebResult<JmidMineResp> {
    let creator = CreatorDao::get_by_user_id(&state.sql_pool, claims.uid()).await?;
    let jmid_prefix = creator.map(|x| x.jmid_prefix);
    ok!(JmidMineResp {jmid_prefix})
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidGetNextResp {
    pub jmid: String,
}

/// Get the next available jmid for this creator.
/// Only available for a creator who had already specified a jm-code
/// # Errors
/// - `jmid_prefix_not_specified`
/// - `jmid_prefix_inactive`
async fn jmid_get_next(
    claims: Claims,
    State(state): State<AppState>,
) -> WebResult<JmidGetNextResp> {
    let creator = CreatorDao::get_by_user_id(&state.sql_pool, claims.uid()).await?
        .ok_or_else(|| common!("jmid_prefix_not_specified", "You have not specified a jmid prefix yet"))?;

    if !creator.active {
        err!("jmid_prefix_inactive", "Your jmid prefix is not active yet, please wait for processing")
    }

    // Count all songs of the creator and add the pending PRs
    let published_songs = SongDao::count_by_user(&state.sql_pool, claims.uid()).await?;
    let pending_prs = SongPublishingReviewDao::count_by_user_and_status(&state.sql_pool, claims.uid(), song_publishing_review::STATUS_PENDING).await?;

    let next_no = published_songs + pending_prs + 1;
    let jmid = format!("{}-{:03}", creator.jmid_prefix, next_no);

    ok!(JmidGetNextResp {jmid})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeJmidReq {
    pub song_id: i64,
    pub old_jmid: String,
    pub new_jmid: String,
}

/// Change jmid of an artwork. The new jmid should match to the creator's jmid, and do not conflict with other songs.
///
/// Only available for a creator who had already specified a jmid prefix
///
/// If a creator had never specified a jmid prefix, it's better to check and initialize the jmid prefix
async fn change_jmid(
    claims: Claims,
    State(state): State<AppState>,
    req: Json<ChangeJmidReq>,
) -> WebResult<()> {
    // 1. Check if the user is a creator
    let guard = state.red_lock.try_lock(&format!("lock:change_jmid:{}", req.song_id)).await?;
    if guard.is_none() {
        err!("operation_in_progress", "Operation in progress")
    }

    let mut song = SongDao::get_by_id(&state.sql_pool, req.song_id).await?
        .ok_or_else(|| common!("not_found", "Song not found"))?;
    if song.uploader_uid != claims.uid() {
        err!("permission_denied", "Permission denied")
    }

    let creator = CreatorDao::get_by_user_id(&state.sql_pool, claims.uid()).await?
        .ok_or_else(|| common!("jmid_prefix_not_specified", "You have not specified a jmid prefix yet"))?;
    if !creator.active {
        err!("jmid_prefix_not_active", "Your jmid prefix is not active yet")
    }
    // 2. Validate the new_jmid format, JM-ABCD-123
    let (prefix, _) = parse_jmid(&req.new_jmid)
        .ok_or_else(|| common!("invalid_jmid", "Invalid jmid format"))?;
    // 3. Check if the new jmid is match to creator's jmid prefix
    if prefix != creator.jmid_prefix {
        err!("jmid_prefix_mismatch", "The jmid prefix must match to yours.")
    }
    // 4. Check if the new jmid is already used
    let available = check_jmid_available(&state.sql_pool, &req.new_jmid).await?;
    if !available {
        err!("jmid_already_in_use", "The jmid({}) is already in use", req.new_jmid)
    };
    // 5. Update the song's jmid, and the corresponding review id
    let mut tx = state.sql_pool.begin().await?;

    let old_jmid = song.display_id;
    song.display_id = req.new_jmid.clone();

    // Update song jmid
    SongDao::update_by_id(&mut *tx, &song).await?;

    // 6. Update the corresponding review's jmid field
    SongPublishingReviewDao::swap_jmid(&mut *tx, &old_jmid, &req.new_jmid).await?;

    tx.commit().await?;

    // 7. Update search index
    search::song::add_or_replace_document(&state.meilisearch, &state.sql_pool, &[song.id]).await?;
    service::recommend_v2::notify_update(song.id, state.redis_conn.clone()).await?;
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityCfg {
    pub contributors: Vec<String>,
}

/// Check whether the `jmid` is available (not used nor locked by other pending SRs).
pub async fn check_jmid_available(
    sql: &PgPool,
    jmid: &str,
) -> anyhow::Result<bool> {
    // Check the songs
    let song = SongDao::get_by_display_id(sql, &jmid).await?;
    if song.is_some() {
        return Ok(false);
    }

    // Check the pending SRs
    // guarantee: If a SR is approved or rejected, the song's display id will be changed to the latest one. So we do not need to check it.
    let prs: Vec<_> = SongPublishingReviewDao::list_by_jmid(sql, jmid).await?;
    let has_pending_prs = prs.iter().any(|x| {
        x.song_display_id == jmid && x.status == 0
    });
    Ok(!has_pending_prs)
}

pub fn parse_jmid(input: &str) -> Option<(&str, &str)> {
    let regex = regex::Regex::new(r"^JM-([A-Z]{3,4})-?(\d{3})$").ok()?;
    let captures = regex.captures(input)?;
    Some((
        captures.get(1)?.as_str(),
        captures.get(2)?.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use crate::web::routes::publish::parse_jmid;

    #[test]
    fn test_parse_jmid() {
        assert_eq!(parse_jmid("JM-ABC-123"), Some(("ABC", "123")));
        assert_eq!(parse_jmid("JM-ABCD-001"), Some(("ABCD", "001")));
        assert_eq!(parse_jmid("JM-ABCD-1"), None);
        assert_eq!(parse_jmid("JM-ABCD-ABC"), None);
        assert_eq!(parse_jmid("JM-A-001"), None);
        assert_eq!(parse_jmid("ABC-123"), None);
        assert_eq!(parse_jmid("ABCD123"), None);
        assert_eq!(parse_jmid("JM-abc-123"), None);
    }
}