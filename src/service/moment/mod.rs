//! Moments: short posts any user can write, with up to [MAX_IMAGES] images and attachments such as
//! a song or a playlist. They show on the square, everyone's newest first, and on their author's
//! page.
//!
//! Everything in a moment is checked before it's shown. Images are checked one by one as they're
//! uploaded ([prepare_image]), so a blocked one can be swapped before posting; one that can't be
//! checked then is kept, marked unchecked. The text, and whatever its attachments need checked, is
//! checked when the moment is posted. If any of that can't be checked, or an image is unchecked,
//! the moment waits in [crate::service::content_review] as a [PendingMoment], seen by no one, its
//! author included, and is saved once everything passes. If something is blocked then, its author
//! is told.

pub mod attachment;

use crate::db::comment::CommentDao;
use crate::db::content_check::ContentCheckDao;
use crate::db::content_review::ContentReviewDao;
use crate::db::moment::{Moment, MomentDao, MomentId, MomentLikeDao};
use crate::db::user::{IUserDao, UserDao, UserId};
use crate::db::CrudDao;
use crate::service::content_check::{self, ContentChecker, Verdict};
use crate::service::content_review::{self, ReviewItem};
use crate::service::errors::{ServiceError, ServiceResult};
use crate::service::notification::{self, to_plain_text, NewNotification};
use crate::service::upload::{scale_down_to_webp, ResizeType};
use crate::web::state::AppState;
use anyhow::anyhow;
use attachment::{AttachmentAdapter, AttachmentKind};
use bytes::Bytes;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use strum::IntoEnumIterator;

pub const CONTENT_MAX_CHARS: usize = 500;
pub const MAX_IMAGES: usize = 9;
/// Attachments a moment can have, of any types. Stored as a list, so allowing more only changes
/// this.
pub const MAX_ATTACHMENTS: usize = 1;
/// Moments a user may post in a minute and in a day, counting ones waiting for a check, and in the
/// day ones blocked by it.
const MOMENTS_PER_MINUTE: i64 = 2;
const MOMENTS_PER_DAY: i64 = 50;
/// The long side of a stored image, and of its thumbnail, at most.
const IMAGE_MAX_SIDE: u32 = 2048;
const THUMB_MAX_SIDE: u32 = 480;
const IMAGE_QUALITY: f32 = 85.0;
/// Uploaded images can be posted for this long.
const UPLOADED_IMAGE_TTL_SECS: u64 = 60 * 60;
/// The target type of comments on moments, as [crate::service::comment::CommentTarget] names it.
pub(crate) const COMMENT_TARGET_TYPE: &str = "moment";
/// How much of a moment its mentions quote.
const QUOTE_CHARS: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum MomentError {
    #[error("moment not found")]
    MomentNotFound,
    #[error("a moment needs text, an image or an attachment")]
    Empty,
    #[error("the text is too long")]
    ContentTooLong,
    #[error("too many images")]
    TooManyImages,
    #[error("too many attachments")]
    TooManyAttachments,
    /// Not uploaded by this user, or uploaded too long ago.
    #[error("image not found, upload it again")]
    ImageNotFound,
    #[error("unknown attachment type")]
    UnknownAttachment,
    /// What it points at doesn't exist or can't be shown to others.
    #[error("invalid attachment")]
    InvalidAttachment,
    /// Blocked by a content check. Users aren't told why.
    #[error("moment contains inappropriate content")]
    Rejected,
    #[error("too many moments, try again later")]
    RateLimited,
    #[error("banned users can't post moments")]
    UserBanned,
}

fn business<T>(e: MomentError) -> ServiceResult<T, MomentError> {
    Err(ServiceError::BusinessError(e))
}

/// An image of a moment as stored, in `moments.images` and in a [PendingMoment].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredImage {
    /// Where the image is stored, for checking it again and for cleaning up.
    pub key: String,
    pub url: String,
    pub thumb_url: String,
    pub width: u32,
    pub height: u32,
}

/// An attachment as stored: its type, and data in the shape that type defines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredAttachment {
    #[serde(rename = "type")]
    pub kind: String,
    pub data: Value,
}

/// @since 261106
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MomentUser {
    pub uid: UserId,
    pub username: String,
    pub avatar_url: Option<String>,
}

/// @since 261106
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MomentImage {
    pub url: String,
    /// At most 480 pixels on the long side, for lists.
    pub thumb_url: String,
    pub width: u32,
    pub height: u32,
}

/// An attachment as shown.
/// @since 261106
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentView {
    /// Such as `song` or `playlist`. Clients show types they don't know as "update to see this".
    #[serde(rename = "type")]
    pub kind: String,
    /// In the shape the type defines: a `song` is the song's public detail, a `playlist` its
    /// metadata. `None` when what it points at can't be seen any more, shown as "unavailable".
    pub data: Option<Value>,
}

/// A moment as shown to one viewer.
/// @since 261106
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MomentItem {
    pub id: MomentId,
    pub author: MomentUser,
    /// May be empty.
    pub content: String,
    pub images: Vec<MomentImage>,
    pub attachments: Vec<AttachmentView>,
    pub like_count: i32,
    /// Whether the viewer liked it.
    pub liked: bool,
    /// Visible comments and replies.
    pub comment_count: i64,
    /// Hidden by a report decision; only its author sees it then.
    pub is_hidden: bool,
    pub create_time: DateTime<Utc>,
}

/// A moment waiting for content checks, kept in its review's payload until everything passes.
///
/// Old payloads stay in the queue across deploys, and a rollback or an older instance still
/// running can read newer ones. So a field added later must have `#[serde(default)]` or be an
/// `Option`, for payloads written before it; and must be safe to lose, as older code ignores it.
/// Never add `#[serde(deny_unknown_fields)]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMoment {
    pub user_id: UserId,
    pub content: String,
    pub images: Vec<StoredImage>,
    /// Keys of the images that couldn't be checked when uploaded, to check with the moment.
    pub unchecked_image_keys: Vec<String>,
    /// Records of the checks already made, of its images when uploaded and of the rest when it
    /// was posted, named after it once it's saved.
    #[serde(default)]
    pub check_record_ids: Vec<i64>,
    pub attachments: Vec<StoredAttachment>,
    /// When it was posted, which it keeps once saved.
    pub create_time: DateTime<Utc>,
}

impl PendingMoment {
    /// The checks it needs: its text, what its attachments need checked, and its unchecked images.
    pub fn review_items(&self) -> anyhow::Result<Vec<ReviewItem>> {
        let mut items = vec![];
        if !self.content.is_empty() {
            items.push(ReviewItem::Text { subject: content_check::SUBJECT_MOMENT_TEXT, text: self.content.clone() });
        }
        for x in &self.attachments {
            let kind = AttachmentKind::parse(&x.kind).ok_or_else(|| anyhow!("Unknown attachment type {}", x.kind))?;
            items.extend(kind.review_items(&x.data)?);
        }
        items.extend(self.unchecked_image_keys.iter().map(|key| ReviewItem::Image {
            subject: content_check::SUBJECT_MOMENT_IMAGE,
            key: key.clone(),
        }));
        Ok(items)
    }
}

/// How an uploaded image did in its content check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageCheck {
    /// Passed; `record_id` is the record of the check, named after the moment once it's saved.
    Passed { record_id: i64 },
    /// Couldn't be checked; checked again with the moment.
    Unchecked,
    /// Checks are off.
    Off,
}

/// An uploaded image, kept in Redis until it's posted.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UploadedImage {
    uid: UserId,
    image: StoredImage,
    /// Passed its content check, or checks are off.
    checked: bool,
    /// The record of the check it passed.
    #[serde(default)]
    check_record_id: Option<i64>,
}

fn uploaded_image_key(image_id: &str) -> String {
    format!("moment:image:{image_id}")
}

/// An image made ready to store: the full image and its thumbnail, as WebP.
pub struct PreparedImage {
    pub webp: Vec<u8>,
    pub thumb: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// `images/moment/<sha1>.webp`, by content, so the same image is stored once.
    pub key: String,
    pub thumb_key: String,
}

/// Scales an uploaded image down to [IMAGE_MAX_SIDE] and [THUMB_MAX_SIDE], keeping its shape. An
/// animated GIF keeps only its first frame. Blocking; run it off the async workers.
pub fn prepare_image(bytes: Bytes) -> anyhow::Result<PreparedImage> {
    let webp = scale_down_to_webp(IMAGE_MAX_SIDE, IMAGE_MAX_SIDE, bytes.clone(), ResizeType::Fit, IMAGE_QUALITY)?;
    let thumb = scale_down_to_webp(THUMB_MAX_SIDE, THUMB_MAX_SIDE, bytes, ResizeType::Fit, IMAGE_QUALITY)?;
    let (width, height) = image::ImageReader::with_format(Cursor::new(&webp), image::ImageFormat::WebP).into_dimensions()?;
    let hash = hex::encode(openssl::sha::sha1(&webp));
    Ok(PreparedImage {
        webp,
        thumb,
        width,
        height,
        key: format!("images/moment/{hash}.webp"),
        thumb_key: format!("images/moment/thumb/{hash}.webp"),
    })
}

/// Keeps an uploaded and stored image for `uid` to post, and returns its id. An unchecked one is
/// checked again with the moment.
pub async fn keep_uploaded_image(state: &AppState, uid: UserId, image: StoredImage, check: ImageCheck) -> anyhow::Result<String> {
    let image_id = uuid::Uuid::new_v4().simple().to_string();
    let (checked, check_record_id) = match check {
        ImageCheck::Passed { record_id } => (true, Some(record_id)),
        ImageCheck::Unchecked => (false, None),
        ImageCheck::Off => (true, None),
    };
    let value = serde_json::to_string(&UploadedImage { uid, image, checked, check_record_id })?;
    let mut redis = state.redis_conn.clone();
    let _: () = redis.set_ex(uploaded_image_key(&image_id), value, UPLOADED_IMAGE_TTL_SECS).await?;
    Ok(image_id)
}

async fn load_uploaded_images(state: &AppState, uid: UserId, image_ids: &[String]) -> ServiceResult<Vec<UploadedImage>, MomentError> {
    let mut redis = state.redis_conn.clone();
    let mut images = vec![];
    for image_id in image_ids {
        let value: Option<String> = redis.get(uploaded_image_key(image_id)).await?;
        let image = value.and_then(|x| serde_json::from_str::<UploadedImage>(&x).ok()).filter(|x| x.uid == uid);
        match image {
            Some(x) => images.push(x),
            None => return business(MomentError::ImageNotFound),
        }
    }
    Ok(images)
}

/// Whether `uid` may post images now. Checked before uploading, so banned users don't spend
/// checks.
pub async fn ensure_can_post(pool: &PgPool, uid: UserId) -> ServiceResult<(), MomentError> {
    let user = UserDao::get_by_id(pool, uid).await?.ok_or_else(|| anyhow!("User {uid} not found"))?;
    if user.is_banned {
        return business(MomentError::UserBanned);
    }
    Ok(())
}

/// An attachment as sent by the client.
/// @since 261106
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentInput {
    #[serde(rename = "type")]
    pub kind: String,
    pub data: Value,
}

/// Posts a moment with images uploaded before, by their ids. Checked first: a blocked one isn't
/// saved, and one that can't be checked now waits to be checked again, seen by no one until it
/// passes, and returns `None`.
pub async fn create(
    state: &AppState,
    uid: UserId,
    content: &str,
    image_ids: &[String],
    attachments: &[AttachmentInput],
) -> ServiceResult<Option<MomentItem>, MomentError> {
    let pool = &state.sql_pool;
    let checker = state.content_checker.as_deref();
    let content = content.trim();
    if content.chars().count() > CONTENT_MAX_CHARS {
        return business(MomentError::ContentTooLong);
    }
    if image_ids.len() > MAX_IMAGES {
        return business(MomentError::TooManyImages);
    }
    if attachments.len() > MAX_ATTACHMENTS {
        return business(MomentError::TooManyAttachments);
    }
    if content.is_empty() && image_ids.is_empty() && attachments.is_empty() {
        return business(MomentError::Empty);
    }
    ensure_can_post(pool, uid).await?;
    let images = load_uploaded_images(state, uid, image_ids).await?;
    let mut stored_attachments = vec![];
    for x in attachments {
        let Some(kind) = AttachmentKind::parse(&x.kind) else {
            return business(MomentError::UnknownAttachment);
        };
        let Some(data) = kind.prepare(state, uid, &x.data).await? else {
            return business(MomentError::InvalidAttachment);
        };
        stored_attachments.push(StoredAttachment { kind: x.kind.clone(), data });
    }

    // Held through counting, checking and saving, or moments posted at once would all see the
    // count from before any of them was saved
    let Some(_lock) = state.red_lock.lock_with_timeout(&format!("moment:create:{uid}"), std::time::Duration::from_secs(10)).await? else {
        return business(MomentError::RateLimited);
    };
    let now = Utc::now().trunc_subsecs(6);
    let subjects = [content_check::SUBJECT_MOMENT_TEXT];
    for (since, limit, count_blocked) in [(now - Duration::minutes(1), MOMENTS_PER_MINUTE, false), (now - Duration::days(1), MOMENTS_PER_DAY, true)] {
        let mut count = MomentDao::count_by_user_after(pool, uid, since).await?
            + ContentReviewDao::count_pending_by_actor_after(pool, uid, &subjects, since).await?;
        if count_blocked && checker.is_some() {
            count += ContentCheckDao::count_blocked_by_actor_and_subject_since(pool, uid, content_check::SUBJECT_MOMENT_TEXT, since).await?;
        }
        if count >= limit {
            return business(MomentError::RateLimited);
        }
    }

    let mut pending = PendingMoment {
        user_id: uid,
        content: content.to_string(),
        images: images.iter().map(|x| x.image.clone()).collect(),
        unchecked_image_keys: if checker.is_some() {
            images.iter().filter(|x| !x.checked).map(|x| x.image.key.clone()).collect()
        } else {
            vec![]
        },
        check_record_ids: images.iter().filter_map(|x| x.check_record_id).collect(),
        attachments: stored_attachments,
        create_time: now,
    };
    let verdict = match checker {
        Some(checker) => {
            let (verdict, record_ids) = check_now(pool, checker, &pending).await?;
            pending.check_record_ids.extend(record_ids);
            verdict
        }
        None => Some(Verdict::Pass),
    };
    let mut tx = pool.begin().await?;
    let saved = match verdict {
        Some(Verdict::Pass) => Some(save(&mut tx, &pending).await?),
        Some(Verdict::Blocked(_)) => return business(MomentError::Rejected),
        None => {
            let payload = serde_json::to_value(&pending)?;
            content_review::enqueue(&mut tx, content_check::SUBJECT_MOMENT_TEXT, uid, payload, now).await?;
            None
        }
    };
    tx.commit().await?;

    // Posted, so they can't be posted again
    let mut redis = state.redis_conn.clone();
    for image_id in image_ids {
        let _: Result<(), _> = redis.del(uploaded_image_key(image_id)).await;
    }
    Ok(match saved {
        Some(moment) => Some(hydrate(state, vec![moment], Some(uid)).await?.remove(0)),
        None => None,
    })
}

/// Checks what can be checked now: the text and what the attachments need. Unchecked images aren't
/// tried again so soon, so a moment with any waits. Returns the records of the checks too.
async fn check_now(pool: &PgPool, checker: &ContentChecker, pending: &PendingMoment) -> anyhow::Result<(Option<Verdict>, Vec<i64>)> {
    let mut unchecked = false;
    let mut record_ids = vec![];
    for item in pending.review_items()? {
        let verdict = match item {
            ReviewItem::Text { subject, text } => {
                let checked = checker.try_check_text(pool, subject, pending.user_id, None, &text).await?;
                record_ids.push(checked.record_id);
                checked.verdict
            }
            ReviewItem::Image { .. } => None,
        };
        match verdict {
            Some(Verdict::Pass) => {}
            Some(blocked @ Verdict::Blocked(_)) => return Ok((Some(blocked), record_ids)),
            None => unchecked = true,
        }
    }
    Ok((if unchecked { None } else { Some(Verdict::Pass) }, record_ids))
}

/// Saves a moment that passed its content checks, or was published without them.
async fn save(tx: &mut sqlx::PgTransaction<'_>, pending: &PendingMoment) -> anyhow::Result<Moment> {
    let mut moment = Moment {
        id: MomentId(0),
        user_id: pending.user_id,
        content: pending.content.clone(),
        images: serde_json::to_value(&pending.images)?,
        attachments: serde_json::to_value(&pending.attachments)?,
        is_hidden: false,
        like_count: 0,
        delete_time: None,
        create_time: pending.create_time,
        update_time: Utc::now(),
    };
    moment.id = MomentDao::insert(&mut **tx, &moment).await?;
    ContentCheckDao::set_target_id(&mut **tx, &pending.check_record_ids, moment.id.0).await?;
    Ok(moment)
}

/// Saves a moment that waited for its content checks, once everything passes, in `tx`, the
/// transaction that ends its review. Returns its new id.
pub async fn publish_pending(tx: &mut sqlx::PgTransaction<'_>, pending: &PendingMoment) -> anyhow::Result<MomentId> {
    save(tx, pending).await.map(|x| x.id)
}

/// Tells the author that a moment that waited for its content checks was blocked and dropped, in
/// `tx`, the transaction that ends its review.
pub async fn reject_pending(tx: &mut sqlx::PgTransaction<'_>, pending: &PendingMoment) -> anyhow::Result<()> {
    let body = if pending.content.is_empty() {
        "你发布的动态没有通过审核，没有发出。".to_string()
    } else {
        format!("你发布的动态「{}」没有通过审核，没有发出。", quote(&pending.content))
    };
    notification::send_notification(tx, NewNotification {
        id: uuid::Uuid::now_v7(),
        recipient_uid: pending.user_id,
        notification_type: "moment.rejected",
        title: "动态没有通过审核".to_string(),
        body,
        content_intent: None,
        occurred_at: Utc::now(),
    }).await?;
    Ok(())
}

/// A moment's text, cut short for mentions.
pub(crate) fn quote(content: &str) -> String {
    let text = to_plain_text(content).replace('\n', " ");
    if text.chars().count() > QUOTE_CHARS {
        format!("{}…", text.chars().take(QUOTE_CHARS).collect::<String>())
    } else {
        text
    }
}

/// Deletes the author's own moment.
pub async fn delete(pool: &PgPool, uid: UserId, moment_id: MomentId) -> ServiceResult<(), MomentError> {
    let Some(moment) = MomentDao::get_by_id(pool, moment_id).await?.filter(|x| x.delete_time.is_none() && x.user_id == uid) else {
        return business(MomentError::MomentNotFound);
    };
    let now = Utc::now();
    MomentDao::update_visibility(pool, moment.id, moment.is_hidden, Some(now), now).await?;
    Ok(())
}

/// Hides or shows a moment, for report decisions.
pub async fn set_hidden(tx: &mut sqlx::PgTransaction<'_>, moment_id: MomentId, hidden: bool) -> anyhow::Result<()> {
    let Some(moment) = MomentDao::get_by_id(&mut **tx, moment_id).await? else {
        return Ok(());
    };
    MomentDao::update_visibility(&mut **tx, moment.id, hidden, moment.delete_time, Utc::now()).await?;
    Ok(())
}

/// Whether `viewer_uid` can see it: everyone if it's visible, its author while it's hidden, no one
/// once it's deleted.
fn visible_to(moment: &Moment, viewer_uid: Option<UserId>) -> bool {
    moment.delete_time.is_none() && (!moment.is_hidden || viewer_uid == Some(moment.user_id))
}

pub async fn detail(state: &AppState, viewer_uid: Option<UserId>, moment_id: MomentId) -> ServiceResult<MomentItem, MomentError> {
    let Some(moment) = MomentDao::get_by_id(&state.sql_pool, moment_id).await?.filter(|x| visible_to(x, viewer_uid)) else {
        return business(MomentError::MomentNotFound);
    };
    Ok(hydrate(state, vec![moment], viewer_uid).await?.remove(0))
}

pub struct MomentPage {
    pub moments: Vec<MomentItem>,
    /// Where to continue for the next page; `None` when there's no more.
    pub next_cursor: Option<MomentCursor>,
}

/// Where a page of moments ended: the `create_time` and `id` of its last moment. Clients get it as
/// an opaque string, `<micros>.<id>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MomentCursor {
    pub time: DateTime<Utc>,
    pub id: MomentId,
}

impl MomentCursor {
    pub fn encode(&self) -> String {
        format!("{}.{}", self.time.timestamp_micros(), self.id)
    }

    pub fn decode(s: &str) -> Option<Self> {
        let (time, id) = s.split_once('.')?;
        Some(MomentCursor {
            time: DateTime::from_timestamp_micros(time.parse().ok()?)?,
            id: MomentId(id.parse().ok()?),
        })
    }
}

/// Takes the page out of `moments`, read with one more than `limit`.
async fn to_page(state: &AppState, mut moments: Vec<Moment>, limit: i64, viewer_uid: Option<UserId>) -> anyhow::Result<MomentPage> {
    let next_cursor = if moments.len() as i64 > limit {
        moments.truncate(limit as usize);
        moments.last().map(|x| MomentCursor { time: x.create_time, id: x.id })
    } else {
        None
    };
    Ok(MomentPage { moments: hydrate(state, moments, viewer_uid).await?, next_cursor })
}

/// Everyone's visible moments, newest first, continuing after `cursor`. Moments of banned users
/// are left out.
pub async fn list_public(state: &AppState, viewer_uid: Option<UserId>, cursor: Option<MomentCursor>, limit: i64) -> anyhow::Result<MomentPage> {
    let moments = MomentDao::page_public(&state.sql_pool, cursor.map(|x| (x.time, x.id)), limit + 1).await?;
    to_page(state, moments, limit, viewer_uid).await
}

/// A user's moments, newest first, continuing after `cursor`. Their hidden ones are shown only to
/// themselves.
pub async fn list_by_user(
    state: &AppState,
    viewer_uid: Option<UserId>,
    user_id: UserId,
    cursor: Option<MomentCursor>,
    limit: i64,
) -> anyhow::Result<MomentPage> {
    let include_hidden = viewer_uid == Some(user_id);
    let moments = MomentDao::page_by_user(&state.sql_pool, user_id, include_hidden, cursor.map(|x| (x.time, x.id)), limit + 1).await?;
    to_page(state, moments, limit, viewer_uid).await
}

/// Likes a visible moment. Liking it again does nothing.
pub async fn like(pool: &PgPool, uid: UserId, moment_id: MomentId) -> ServiceResult<(), MomentError> {
    if !MomentDao::get_by_id(pool, moment_id).await?.is_some_and(|x| x.is_visible()) {
        return business(MomentError::MomentNotFound);
    }
    let mut tx = pool.begin().await?;
    if MomentLikeDao::insert(&mut *tx, moment_id, uid, Utc::now()).await? {
        MomentDao::add_like_count(&mut *tx, moment_id, 1).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Takes a like back. Doing it again does nothing.
pub async fn unlike(pool: &PgPool, uid: UserId, moment_id: MomentId) -> ServiceResult<(), MomentError> {
    let mut tx = pool.begin().await?;
    if MomentLikeDao::delete(&mut *tx, moment_id, uid).await? {
        MomentDao::add_like_count(&mut *tx, moment_id, -1).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Hydrates moments into what `viewer_uid` sees: their authors, whether the viewer liked them, how
/// many comments they have and their attachments, a query or so each for the whole batch.
pub(crate) async fn hydrate(state: &AppState, moments: Vec<Moment>, viewer_uid: Option<UserId>) -> anyhow::Result<Vec<MomentItem>> {
    let pool = &state.sql_pool;
    let ids: Vec<MomentId> = moments.iter().map(|x| x.id).collect();
    let mut user_ids: Vec<UserId> = moments.iter().map(|x| x.user_id).collect();
    user_ids.sort();
    user_ids.dedup();
    let users: HashMap<UserId, MomentUser> = UserDao::list_by_ids(pool, &user_ids).await?
        .into_iter()
        .map(|x| (x.id, MomentUser { uid: x.id, username: x.username, avatar_url: x.avatar_url }))
        .collect();
    let liked: HashSet<MomentId> = match viewer_uid {
        Some(uid) => MomentLikeDao::list_liked_by_user(pool, uid, &ids).await?.into_iter().collect(),
        None => HashSet::new(),
    };
    let raw_ids: Vec<i64> = ids.iter().map(|x| x.0).collect();
    let comment_counts: HashMap<i64, i64> = CommentDao::count_visible_by_targets(pool, COMMENT_TARGET_TYPE, &raw_ids).await?
        .into_iter()
        .collect();

    let images: Vec<Vec<StoredImage>> = moments.iter()
        .map(|x| serde_json::from_value(x.images.clone()))
        .collect::<Result<_, _>>()?;
    let attachments: Vec<Vec<StoredAttachment>> = moments.iter()
        .map(|x| serde_json::from_value(x.attachments.clone()))
        .collect::<Result<_, _>>()?;
    let views = show_attachments(state, &attachments).await?;

    Ok(moments.into_iter().zip(images).zip(views).map(|((x, images), attachments)| MomentItem {
        id: x.id,
        author: users.get(&x.user_id).cloned().unwrap_or_else(|| MomentUser {
            uid: x.user_id,
            username: "Unknown User".to_string(),
            avatar_url: None,
        }),
        content: x.content,
        images: images.into_iter().map(|x| MomentImage { url: x.url, thumb_url: x.thumb_url, width: x.width, height: x.height }).collect(),
        attachments,
        like_count: x.like_count,
        liked: liked.contains(&x.id),
        comment_count: comment_counts.get(&x.id.0).copied().unwrap_or(0),
        is_hidden: x.is_hidden,
        create_time: x.create_time,
    }).collect())
}

/// Shows the attachments of several moments, each type's all at once.
async fn show_attachments(state: &AppState, attachments: &[Vec<StoredAttachment>]) -> anyhow::Result<Vec<Vec<AttachmentView>>> {
    let mut views: Vec<Vec<AttachmentView>> = attachments.iter()
        .map(|x| x.iter().map(|x| AttachmentView { kind: x.kind.clone(), data: None }).collect())
        .collect();
    for kind in AttachmentKind::iter() {
        let mut places = vec![];
        let mut data = vec![];
        for (i, list) in attachments.iter().enumerate() {
            for (j, x) in list.iter().enumerate() {
                if x.kind == kind.name() {
                    places.push((i, j));
                    data.push(&x.data);
                }
            }
        }
        if data.is_empty() {
            continue;
        }
        for ((i, j), shown) in places.into_iter().zip(kind.show_many(state, &data).await?) {
            views[i][j].data = shown;
        }
    }
    Ok(views)
}
