use crate::db::moment::MomentId;
use crate::db::user::UserId;
use crate::service::content_check::{self, ImageUpload, Verdict};
use crate::service::errors::ServiceError;
use crate::service::moment::{self, AttachmentInput, MomentCursor, MomentError, MomentItem, StoredImage};
use crate::web::jwt::{Claims, OptionalClaims};
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::state::AppState;
use crate::{common, err, ok};
use anyhow::Context;
use async_backtrace::framed;
use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use image::{ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

const IMAGE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Moments, short posts any user can write. See [crate::service::moment]. The square lists them
/// under `/feed`.
/// @since 261106
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/upload_image", post(upload_image).layer(DefaultBodyLimit::max(10 * 1024 * 1024)))
        .route("/create", post(create))
        .route("/delete", post(delete))
        .route("/detail", get(detail))
        .route("/list_by_user", get(list_by_user))
        .route("/like", post(like))
        .route("/unlike", post(unlike))
}

/// @since 261106
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadImageResp {
    /// Pass it to `/moment/create` within an hour.
    pub image_id: String,
    pub url: String,
    pub thumb_url: String,
    pub width: u32,
    pub height: u32,
}

/// Uploads one image for a moment, as a multipart body with the file as its only field: PNG, JPEG,
/// WebP, AVIF or GIF (only the first frame is kept), up to 8 MB. Checked here: one the check
/// blocks errors with `image_rejected`, so it can be swapped before posting. Up to 30 images an
/// hour are checked for each user; past that it errors with `rate_limited`.
#[framed]
async fn upload_image(claims: Claims, state: State<AppState>, mut multipart: Multipart) -> WebResult<UploadImageResp> {
    let uid = claims.uid();
    moment::ensure_can_post(&state.sql_pool, uid).await?;
    let field = multipart.next_field().await?.with_context(|| "No data field found")?;
    let bytes = field.bytes().await?;
    if bytes.len() > IMAGE_MAX_BYTES {
        err!("image_too_large", "Image size must be less than 8MB");
    }
    let format = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .ok()
        .and_then(|x| x.format());
    if !matches!(format, Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Avif | ImageFormat::Gif)) {
        err!("invalid_image", "The image is not supported");
    }

    // Off the async workers: encoding takes long enough to stall other requests
    let prepared = tokio::task::spawn_blocking({
        let bytes: Bytes = bytes.clone();
        move || moment::prepare_image(bytes)
    }).await.context("Failed to prepare the image")?
        .map_err(|_| common!("invalid_image", "The image is not supported"))?;

    // Held until the image is stored, so the user's uploads are checked one at a time
    let (checked, _lock) = match &state.content_checker {
        Some(checker) => {
            let (verdict, lock) = content_check::try_check_image_upload(checker, &state, ImageUpload {
                subject: content_check::SUBJECT_MOMENT_IMAGE,
                actor_uid: uid,
                target_id: 0,
                uploaded: &bytes,
                webp: &prepared.webp,
                key: &prepared.key,
                blocked_dir: "moment",
            }).await?;
            match verdict {
                Some(Verdict::Pass) => (true, Some(lock)),
                Some(Verdict::Blocked(_)) => err!("image_rejected", "Image contains inappropriate content"),
                // Kept, and checked again with the moment
                None => (false, Some(lock)),
            }
        }
        None => (true, None),
    };

    let image = state.file_host.upload(Bytes::from(prepared.webp), &prepared.key).await?;
    let thumb = state.file_host.upload(Bytes::from(prepared.thumb), &prepared.thumb_key).await?;
    let stored = StoredImage {
        key: prepared.key,
        url: image.public_url,
        thumb_url: thumb.public_url,
        width: prepared.width,
        height: prepared.height,
    };
    let image_id = moment::keep_uploaded_image(&state, uid, stored.clone(), checked).await?;
    ok!(UploadImageResp { image_id, url: stored.url, thumb_url: stored.thumb_url, width: stored.width, height: stored.height })
}

/// @since 261106
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateReq {
    /// Up to 500 characters, trimmed. May be empty when there are images or attachments.
    #[serde(default)]
    pub content: String,
    /// From `/moment/upload_image`, in order. Up to 9.
    #[serde(default)]
    pub image_ids: Vec<String>,
    /// `{ type, data }` each. For now up to 1, a `song` (`{ "id": <song id> }`) or a `playlist`
    /// (`{ "id": <playlist id> }`, public).
    #[serde(default)]
    pub attachments: Vec<AttachmentInput>,
}

/// @since 261106
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateResp {
    /// The new moment; `None` if it couldn't be checked now and waits to be checked again. It's
    /// seen by no one, its author included, until it passes; if it's blocked then, its author gets
    /// a notification.
    pub moment: Option<MomentItem>,
}

/// Posts a moment. One the check blocks isn't saved and errors with `moment_rejected`.
#[framed]
async fn create(claims: Claims, state: State<AppState>, req: Json<CreateReq>) -> WebResult<CreateResp> {
    let moment = moment::create(&state, claims.uid(), &req.content, &req.image_ids, &req.attachments).await?;
    ok!(CreateResp { moment })
}

/// @since 261106
#[derive(Debug, Serialize, Deserialize)]
pub struct MomentIdReq {
    pub moment_id: MomentId,
}

/// Deletes the user's own moment.
#[framed]
async fn delete(claims: Claims, state: State<AppState>, req: Json<MomentIdReq>) -> WebResult<()> {
    moment::delete(&state.sql_pool, claims.uid(), req.moment_id).await?;
    ok!(())
}

/// A moment. Its author sees it while it's hidden too.
#[framed]
async fn detail(claims: OptionalClaims, state: State<AppState>, req: Query<MomentIdReq>) -> WebResult<MomentItem> {
    let moment = moment::detail(&state, claims.0.map(|x| x.uid()), req.moment_id).await?;
    ok!(moment)
}

/// @since 261106
#[derive(Debug, Serialize, Deserialize)]
pub struct ListByUserReq {
    pub user_id: UserId,
    /// `next_cursor` of the previous page; omit for the first page.
    pub cursor: Option<String>,
    /// 1..=50, defaults to 20.
    pub limit: Option<i64>,
}

/// @since 261106
#[derive(Debug, Serialize, Deserialize)]
pub struct ListResp {
    /// Newest first.
    pub moments: Vec<MomentItem>,
    /// Pass it back as `cursor` for the next page; `None` when there's no more.
    pub next_cursor: Option<String>,
}

/// A user's moments, newest first, for their page. Their hidden ones are shown only to themselves.
#[framed]
async fn list_by_user(claims: OptionalClaims, state: State<AppState>, req: Query<ListByUserReq>) -> WebResult<ListResp> {
    let cursor = match req.cursor.as_deref() {
        None => None,
        Some(x) => match MomentCursor::decode(x) {
            Some(x) => Some(x),
            None => err!("invalid_cursor", "Invalid cursor"),
        },
    };
    let page = moment::list_by_user(&state, claims.0.map(|x| x.uid()), req.user_id, cursor, req.limit.unwrap_or(20).clamp(1, 50)).await?;
    ok!(ListResp { moments: page.moments, next_cursor: page.next_cursor.map(|x| x.encode()) })
}

#[framed]
async fn like(claims: Claims, state: State<AppState>, req: Json<MomentIdReq>) -> WebResult<()> {
    moment::like(&state.sql_pool, claims.uid(), req.moment_id).await?;
    ok!(())
}

#[framed]
async fn unlike(claims: Claims, state: State<AppState>, req: Json<MomentIdReq>) -> WebResult<()> {
    moment::unlike(&state.sql_pool, claims.uid(), req.moment_id).await?;
    ok!(())
}

impl From<ServiceError<MomentError>> for WebError<CommonError> {
    fn from(err: ServiceError<MomentError>) -> Self {
        match err {
            ServiceError::BusinessError(e) => {
                let code = match e {
                    MomentError::MomentNotFound => "moment_not_found",
                    MomentError::Empty => "moment_empty",
                    MomentError::ContentTooLong => "content_too_long",
                    MomentError::TooManyImages => "too_many_images",
                    MomentError::TooManyAttachments => "too_many_attachments",
                    MomentError::ImageNotFound => "image_not_found",
                    MomentError::UnknownAttachment => "unknown_attachment",
                    MomentError::InvalidAttachment => "invalid_attachment",
                    MomentError::Rejected => "moment_rejected",
                    MomentError::RateLimited => "rate_limited",
                    MomentError::UserBanned => "user_banned",
                };
                common!(code, "{}", e)
            }
            ServiceError::Other(e) => WebError::Internal(e),
        }
    }
}
