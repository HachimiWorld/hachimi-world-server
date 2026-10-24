//! Comments on songs, and the replies under them. A comment has two levels: a top-level comment on
//! the target, and replies under it; a reply to a reply stays under the same top-level comment and
//! names whom it replies to.
//!
//! The comments table holds what every kind of comment has. What differs by target, such as who
//! owns it and who can see it, is in [CommentTarget]; something only one kind of target needs goes
//! in a table of that kind, keyed by comment id.
//!
//! New comments are checked before they're published. One that can't be checked then waits as
//! pending, seen only by its author, and [crate::service::content_review] checks it again.

use crate::db::comment::{Comment, CommentDao, CommentId, CommentLikeDao, STATUS_PENDING, STATUS_PUBLISHED, STATUS_REJECTED};
use crate::db::content_check::ContentCheckDao;
use crate::db::song::{ISongDao, SongDao, SongId};
use crate::db::user::{IUserDao, UserDao, UserId};
use crate::db::CrudDao;
use crate::service::content_check::{self, ContentChecker, Verdict};
use crate::service::content_review;
use crate::service::errors::{ServiceError, ServiceResult};
use crate::service::ugc::{self, UgcPreview};
use crate::util::redlock::RedLock;
use anyhow::anyhow;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use strum::{EnumIter, IntoEnumIterator};

const CONTENT_MAX_CHARS: usize = 500;
/// How many comments a user may post in an hour, counting ones blocked by content checks, so
/// rewording a blocked comment again and again to feel out the thresholds is limited too.
const COMMENTS_PER_HOUR: i64 = 30;
/// Replies shown under each top-level comment in a list.
const FIRST_REPLIES: i64 = 3;

/// What a comment can be on. Each says who owns its targets and who can see them.
/// @since 261021
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter)]
pub enum CommentTarget {
    Song,
}

/// A target as comments need it, whatever its kind.
pub struct TargetInfo {
    pub owner_uid: UserId,
    /// Hidden by the platform: only the owner can see it and its comments.
    pub is_hidden: bool,
    pub preview: UgcPreview,
}

impl TargetInfo {
    fn visible_to(&self, viewer_uid: Option<UserId>) -> bool {
        !self.is_hidden || viewer_uid == Some(self.owner_uid)
    }
}

impl CommentTarget {
    pub fn parse(s: &str) -> Option<Self> {
        Self::iter().find(|x| x.as_str() == s)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Song => "song",
        }
    }

    /// The content check subject of comments on this kind of target.
    pub fn subject(self) -> &'static str {
        match self {
            Self::Song => content_check::SUBJECT_SONG_COMMENT,
        }
    }

    pub async fn load(self, pool: &PgPool, target_id: i64) -> sqlx::Result<Option<TargetInfo>> {
        Ok(self.load_many(pool, &[target_id]).await?.remove(&target_id))
    }

    /// The targets of these ids that exist, by id.
    pub async fn load_many(self, pool: &PgPool, target_ids: &[i64]) -> sqlx::Result<HashMap<i64, TargetInfo>> {
        match self {
            Self::Song => {
                let ids: Vec<SongId> = target_ids.iter().map(|x| SongId(*x)).collect();
                Ok(SongDao::list_by_ids(pool, &ids).await?.into_iter().map(|x| (x.id.0, TargetInfo {
                    owner_uid: x.uploader_uid,
                    is_hidden: x.is_hidden,
                    preview: ugc::song_preview(&x),
                })).collect())
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommentError {
    #[error("target not found")]
    TargetNotFound,
    #[error("comment not found")]
    CommentNotFound,
    #[error("comment is empty")]
    ContentEmpty,
    #[error("comment is too long")]
    ContentTooLong,
    /// Blocked by a content check. Users aren't told why.
    #[error("comment contains inappropriate content")]
    Rejected,
    #[error("too many comments, try again later")]
    RateLimited,
    #[error("banned users can't comment")]
    UserBanned,
    #[error("permission denied")]
    PermissionDenied,
}

fn business<T>(e: CommentError) -> ServiceResult<T, CommentError> {
    Err(ServiceError::BusinessError(e))
}

/// Published, not hidden and not deleted: seen by everyone who can see the target.
fn is_public(comment: &Comment) -> bool {
    comment.status == STATUS_PUBLISHED && !comment.is_hidden && comment.delete_time.is_none()
}

/// Seen by `viewer_uid` with its content: public, or their own and not deleted.
fn is_shown_to(comment: &Comment, viewer_uid: Option<UserId>) -> bool {
    is_public(comment) || (comment.delete_time.is_none() && viewer_uid == Some(comment.user_id))
}

/// @since 261021
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentUser {
    pub uid: UserId,
    pub username: String,
    pub avatar_url: Option<String>,
}

/// A comment as shown to one viewer.
/// @since 261021
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentItem {
    pub id: CommentId,
    pub target_type: String,
    pub target_id: i64,
    /// `None` for a top-level comment.
    pub root_id: Option<CommentId>,
    /// `None` for a top-level comment.
    pub reply_to_id: Option<CommentId>,
    /// Whom a reply to another reply answers; `None` for a top-level comment and a reply to it.
    pub reply_to_user: Option<CommentUser>,
    /// `None` when `removed`.
    pub author: Option<CommentUser>,
    /// Empty when `removed`.
    pub content: String,
    /// `published`; `pending` or `rejected` only for the viewer's own comments.
    pub status: String,
    /// Hidden by a report decision; only the viewer's own comments show it.
    pub is_hidden: bool,
    /// Deleted or hidden, kept only so its replies have a place. Shown as "this comment was removed".
    pub removed: bool,
    pub like_count: i32,
    /// Whether the viewer liked it.
    pub liked: bool,
    pub create_time: DateTime<Utc>,
}

/// A top-level comment and the first few replies under it.
/// @since 261021
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentThread {
    pub comment: CommentItem,
    /// Oldest first.
    pub replies: Vec<CommentItem>,
    /// All replies the viewer can see.
    pub reply_count: i64,
}

/// Turns comments into what `viewer_uid` sees, looking up the users and likes they need.
pub(crate) async fn to_items(pool: &PgPool, comments: Vec<Comment>, viewer_uid: Option<UserId>) -> sqlx::Result<Vec<CommentItem>> {
    // The reply-to user is only named for replies to other replies
    let reply_to_uids: HashMap<CommentId, UserId> = comments.iter()
        .filter(|x| x.reply_to_id.is_some() && x.reply_to_id != x.root_id)
        .map(|x| (x.id, x.reply_to_uid))
        .collect();
    let mut user_ids: Vec<UserId> = comments.iter().map(|x| x.user_id).chain(reply_to_uids.values().copied()).collect();
    user_ids.sort();
    user_ids.dedup();
    let users: HashMap<UserId, CommentUser> = UserDao::list_by_ids(pool, &user_ids).await?
        .into_iter()
        .map(|x| (x.id, CommentUser { uid: x.id, username: x.username, avatar_url: x.avatar_url }))
        .collect();
    let liked: HashSet<CommentId> = match viewer_uid {
        Some(uid) => {
            let ids: Vec<CommentId> = comments.iter().map(|x| x.id).collect();
            CommentLikeDao::list_liked_by_user(pool, uid, &ids).await?.into_iter().collect()
        }
        None => HashSet::new(),
    };

    Ok(comments.into_iter().map(|x| {
        let removed = !is_shown_to(&x, viewer_uid);
        CommentItem {
            id: x.id,
            target_type: x.target_type,
            target_id: x.target_id,
            root_id: x.root_id,
            reply_to_id: x.reply_to_id,
            reply_to_user: reply_to_uids.get(&x.id).and_then(|uid| users.get(uid)).cloned(),
            author: if removed { None } else { users.get(&x.user_id).cloned() },
            content: if removed { String::new() } else { x.content },
            status: if removed { STATUS_PUBLISHED.to_string() } else { x.status },
            is_hidden: !removed && x.is_hidden,
            removed,
            like_count: x.like_count,
            liked: liked.contains(&x.id),
            create_time: x.create_time,
        }
    }).collect())
}

async fn load_visible_target(pool: &PgPool, target: CommentTarget, target_id: i64, viewer_uid: Option<UserId>) -> ServiceResult<TargetInfo, CommentError> {
    match target.load(pool, target_id).await? {
        Some(x) if x.visible_to(viewer_uid) => Ok(x),
        _ => business(CommentError::TargetNotFound),
    }
}

pub struct CommentPage {
    /// Published comments and replies on the target.
    pub total: i64,
    pub threads: Vec<CommentThread>,
}

/// Top-level comments on a target, most liked first if `by_likes`, else newest first.
pub async fn list(
    pool: &PgPool,
    viewer_uid: Option<UserId>,
    target: CommentTarget,
    target_id: i64,
    by_likes: bool,
    page_index: i64,
    page_size: i64,
) -> ServiceResult<CommentPage, CommentError> {
    load_visible_target(pool, target, target_id, viewer_uid).await?;
    let roots = CommentDao::page_roots(pool, target.as_str(), target_id, viewer_uid, by_likes, page_size, page_index * page_size).await?;
    let root_ids: Vec<CommentId> = roots.iter().map(|x| x.id).collect();
    let replies = CommentDao::list_first_replies(pool, &root_ids, viewer_uid, FIRST_REPLIES).await?;
    let reply_counts: HashMap<CommentId, i64> = CommentDao::count_replies(pool, &root_ids, viewer_uid).await?
        .into_iter()
        .map(|x| (x.root_id, x.count))
        .collect();
    let total = CommentDao::count_published_by_target(pool, target.as_str(), target_id).await?;

    let mut replies_by_root: HashMap<CommentId, Vec<CommentItem>> = HashMap::new();
    for reply in to_items(pool, replies, viewer_uid).await? {
        if let Some(root_id) = reply.root_id {
            replies_by_root.entry(root_id).or_default().push(reply);
        }
    }
    let threads = to_items(pool, roots, viewer_uid).await?.into_iter().map(|comment| CommentThread {
        replies: replies_by_root.remove(&comment.id).unwrap_or_default(),
        reply_count: reply_counts.get(&comment.id).copied().unwrap_or(0),
        comment,
    }).collect();
    Ok(CommentPage { total, threads })
}

pub struct ReplyPage {
    /// All replies under the root the viewer can see.
    pub total: i64,
    /// Oldest first.
    pub replies: Vec<CommentItem>,
}

/// Replies under a top-level comment, oldest first.
pub async fn list_replies(
    pool: &PgPool,
    viewer_uid: Option<UserId>,
    root_id: CommentId,
    page_index: i64,
    page_size: i64,
) -> ServiceResult<ReplyPage, CommentError> {
    let Some(root) = CommentDao::get_by_id(pool, root_id).await?.filter(|x| x.root_id.is_none()) else {
        return business(CommentError::CommentNotFound);
    };
    let Some(target) = CommentTarget::parse(&root.target_type) else {
        return business(CommentError::CommentNotFound);
    };
    load_visible_target(pool, target, root.target_id, viewer_uid).await?;
    let replies = CommentDao::page_replies(pool, root_id, viewer_uid, page_size, page_index * page_size).await?;
    let total = CommentDao::count_replies(pool, &[root_id], viewer_uid).await?.first().map(|x| x.count).unwrap_or(0);
    Ok(ReplyPage { total, replies: to_items(pool, replies, viewer_uid).await? })
}

/// Posts a comment on a target, or a reply to `reply_to_id`. Checked first: blocked ones aren't
/// saved, and ones that can't be checked now wait as pending.
#[allow(clippy::too_many_arguments)]
pub async fn create(
    pool: &PgPool,
    red_lock: &RedLock,
    checker: Option<&ContentChecker>,
    uid: UserId,
    target: CommentTarget,
    target_id: i64,
    content: &str,
    reply_to_id: Option<CommentId>,
) -> ServiceResult<CommentItem, CommentError> {
    let content = content.trim();
    if content.is_empty() {
        return business(CommentError::ContentEmpty);
    }
    if content.chars().count() > CONTENT_MAX_CHARS {
        return business(CommentError::ContentTooLong);
    }
    let user = UserDao::get_by_id(pool, uid).await?.ok_or_else(|| anyhow!("User {uid} not found"))?;
    if user.is_banned {
        return business(CommentError::UserBanned);
    }
    let target_info = load_visible_target(pool, target, target_id, Some(uid)).await?;
    let (root_id, reply_to_uid) = match reply_to_id {
        None => (None, target_info.owner_uid),
        Some(reply_to_id) => {
            let parent = CommentDao::get_by_id(pool, reply_to_id).await?
                .filter(|x| is_public(x) && x.target_type == target.as_str() && x.target_id == target_id);
            let Some(parent) = parent else {
                return business(CommentError::CommentNotFound);
            };
            (Some(parent.root_id.unwrap_or(parent.id)), parent.user_id)
        }
    };

    // Held through counting, checking and saving, or comments posted at once would all see the
    // count from before any of them was saved
    let Some(_lock) = red_lock.lock_with_timeout(&format!("comment:create:{uid}"), std::time::Duration::from_secs(10)).await? else {
        return business(CommentError::RateLimited);
    };
    let now = Utc::now().trunc_subsecs(6);
    let since = now - Duration::hours(1);
    let mut count = CommentDao::count_by_user_after(pool, uid, since).await?;
    if checker.is_some() {
        count += ContentCheckDao::count_blocked_by_actor_and_subject_since(pool, uid, target.subject(), since).await?;
    }
    if count >= COMMENTS_PER_HOUR {
        return business(CommentError::RateLimited);
    }

    let mut comment = Comment {
        id: CommentId(0),
        target_type: target.as_str().to_string(),
        target_id,
        user_id: uid,
        root_id,
        reply_to_id,
        reply_to_uid,
        content: content.to_string(),
        status: STATUS_PUBLISHED.to_string(),
        is_hidden: false,
        like_count: 0,
        delete_time: None,
        create_time: now,
        update_time: now,
    };
    // Inserted first so the check can name the comment, and rolled back if it's blocked. A blocked
    // comment's check keeps an id that is never used.
    let mut tx = pool.begin().await?;
    comment.id = CommentDao::insert(&mut *tx, &comment).await?;
    if let Some(checker) = checker {
        match checker.try_check_text(pool, target.subject(), uid, comment.id.0, content).await? {
            Some(Verdict::Pass) => {}
            Some(Verdict::Blocked(_)) => {
                tx.rollback().await?;
                return business(CommentError::Rejected);
            }
            None => {
                comment.status = STATUS_PENDING.to_string();
                CommentDao::update_status(&mut *tx, comment.id, STATUS_PENDING, now).await?;
                content_review::enqueue(&mut tx, target.subject(), comment.id.0, now).await?;
            }
        }
    }
    tx.commit().await?;
    Ok(to_items(pool, vec![comment], Some(uid)).await?.remove(0))
}

/// Deletes a comment. Its author can, and so can the target's owner. Replies under a deleted
/// top-level comment stay.
pub async fn delete(pool: &PgPool, uid: UserId, comment_id: CommentId) -> ServiceResult<(), CommentError> {
    let Some(comment) = CommentDao::get_by_id(pool, comment_id).await?.filter(|x| x.delete_time.is_none()) else {
        return business(CommentError::CommentNotFound);
    };
    if comment.user_id != uid {
        let owner_uid = match CommentTarget::parse(&comment.target_type) {
            Some(target) => target.load(pool, comment.target_id).await?.map(|x| x.owner_uid),
            None => None,
        };
        if owner_uid != Some(uid) {
            return business(CommentError::PermissionDenied);
        }
    }
    CommentDao::update_delete_time(pool, comment_id, Utc::now()).await?;
    Ok(())
}

/// Likes a public comment. Liking it again does nothing.
pub async fn like(pool: &PgPool, uid: UserId, comment_id: CommentId) -> ServiceResult<(), CommentError> {
    if !CommentDao::get_by_id(pool, comment_id).await?.is_some_and(|x| is_public(&x)) {
        return business(CommentError::CommentNotFound);
    }
    let mut tx = pool.begin().await?;
    match CommentLikeDao::insert(&mut *tx, comment_id, uid, Utc::now()).await {
        Ok(()) => {}
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return Ok(()),
        Err(e) => return Err(e.into()),
    }
    CommentDao::add_like_count(&mut *tx, comment_id, 1).await?;
    tx.commit().await?;
    Ok(())
}

/// Takes a like back. Doing it again does nothing.
pub async fn unlike(pool: &PgPool, uid: UserId, comment_id: CommentId) -> ServiceResult<(), CommentError> {
    let mut tx = pool.begin().await?;
    if CommentLikeDao::delete(&mut *tx, comment_id, uid).await? {
        CommentDao::add_like_count(&mut *tx, comment_id, -1).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The author and text of a comment waiting for a content check; `None` if it no longer waits,
/// such as when it was deleted.
pub async fn load_pending(pool: &PgPool, comment_id: CommentId) -> sqlx::Result<Option<(UserId, String)>> {
    Ok(CommentDao::get_by_id(pool, comment_id).await?
        .filter(|x| x.status == STATUS_PENDING && x.delete_time.is_none())
        .map(|x| (x.user_id, x.content)))
}

/// Publishes a pending comment, or rejects it.
pub async fn settle_pending(pool: &PgPool, comment_id: CommentId, publish: bool) -> sqlx::Result<()> {
    let status = if publish { STATUS_PUBLISHED } else { STATUS_REJECTED };
    CommentDao::update_status(pool, comment_id, status, Utc::now()).await
}

/// Hides or shows a comment, for report decisions.
pub async fn set_hidden(tx: &mut sqlx::PgTransaction<'_>, comment_id: CommentId, hidden: bool) -> sqlx::Result<()> {
    CommentDao::update_hidden(&mut **tx, comment_id, hidden, Utc::now()).await
}

/// What a received reply is on.
/// @since 261021
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyTarget {
    /// Such as `song`.
    pub target_type: String,
    pub target_id: i64,
    pub preview: UgcPreview,
}

/// A comment replying to the user, or on what they own.
/// @since 261021
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedReplyItem {
    pub comment: CommentItem,
    pub target: ReplyTarget,
    /// The user's comment it replies to; `None` for a comment on what they own, or if that comment
    /// was deleted.
    pub replied_content: Option<String>,
}

/// How many comments replying to `uid` or on what they own others published after `after`.
pub async fn count_received_replies_after(pool: &PgPool, uid: UserId, after: DateTime<Utc>) -> sqlx::Result<i64> {
    CommentDao::count_published_replies_to_after(pool, uid, after).await
}

/// Comments replying to `uid` or on what they own, by others, newest first. Ones on targets `uid`
/// can't see are left out. Returns the page and whether there are more.
pub async fn received_replies(
    pool: &PgPool,
    uid: UserId,
    before: Option<(DateTime<Utc>, CommentId)>,
    limit: i64,
) -> sqlx::Result<(Vec<ReceivedReplyItem>, bool)> {
    let mut comments = CommentDao::list_published_replies_to(pool, uid, before, limit + 1).await?;
    let has_more = comments.len() as i64 > limit;
    comments.truncate(limit as usize);

    let mut targets: HashMap<(CommentTarget, i64), TargetInfo> = HashMap::new();
    for target in CommentTarget::iter() {
        let ids: Vec<i64> = comments.iter()
            .filter(|x| CommentTarget::parse(&x.target_type) == Some(target))
            .map(|x| x.target_id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        for (id, info) in target.load_many(pool, &ids).await? {
            if info.visible_to(Some(uid)) {
                targets.insert((target, id), info);
            }
        }
    }
    let replied_ids: Vec<CommentId> = comments.iter().filter_map(|x| x.reply_to_id).collect();
    let replied: HashMap<CommentId, String> = CommentDao::list_by_ids(pool, &replied_ids).await?
        .into_iter()
        .filter(|x| x.delete_time.is_none())
        .map(|x| (x.id, x.content))
        .collect();

    let items = to_items(pool, comments, Some(uid)).await?
        .into_iter()
        .filter_map(|comment| {
            let kind = CommentTarget::parse(&comment.target_type)?;
            let info = targets.get(&(kind, comment.target_id))?;
            Some(ReceivedReplyItem {
                target: ReplyTarget {
                    target_type: comment.target_type.clone(),
                    target_id: comment.target_id,
                    preview: info.preview.clone(),
                },
                replied_content: comment.reply_to_id.and_then(|x| replied.get(&x).cloned()),
                comment,
            })
        })
        .collect();
    Ok((items, has_more))
}
