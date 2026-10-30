//! Comments on songs and posts, and the replies under them. A comment has two levels: a top-level
//! comment on the target, and replies under it; a reply to a reply stays under the same top-level
//! comment and names whom it replies to.
//!
//! The comments table holds what every kind of comment has. What differs by target, such as who
//! owns it and who can see it, is in [CommentTarget]; something only one kind of target needs goes
//! in a table of that kind, keyed by comment id.
//!
//! New comments are checked before they're saved. One that can't be checked then waits in
//! [crate::service::content_review] as a [PendingComment], seen by no one, its author included,
//! and is saved once it passes. Saved comments are seen by everyone who can see their target,
//! unless hidden by a report decision or deleted.

use crate::db::comment::{Comment, CommentDao, CommentId, CommentLikeDao};
use crate::db::content_check::ContentCheckDao;
use crate::db::content_review::ContentReviewDao;
use crate::db::user::{IUserDao, UserDao, UserId};
use crate::db::CrudDao;
use crate::service::content_check::{ContentChecker, Verdict};
use crate::service::content_review;
use crate::service::errors::{ServiceError, ServiceResult};
use crate::service::notification::ContentIntent;
use crate::service::ugc::UgcPreview;
use crate::util::redlock::RedLock;
use enum_dispatch::enum_dispatch;
use anyhow::anyhow;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use strum::{EnumIter, IntoEnumIterator};

mod post;
mod song;

const CONTENT_MAX_CHARS: usize = 500;
/// How many comments a user may post in an hour, counting ones blocked by content checks, so
/// rewording a blocked comment again and again to feel out the thresholds is limited too.
const COMMENTS_PER_HOUR: i64 = 30;
/// Replies shown under each top-level comment in a list.
const FIRST_REPLIES: i64 = 3;
/// Unread replies are counted up to this, so a summary stays cheap for someone who hasn't read
/// theirs in a long time. Badges show more than 99 as "99+".
pub const MAX_UNREAD_REPLIES: i64 = 100;

/// What a comment can be on. Each variant carries the kind's adapter (`song.rs`, `post.rs`), and
/// `enum_dispatch` forwards [CommentTargetAdapter] to it, as [crate::service::ugc::UgcKind] does
/// for reports. Adding a kind means a new adapter and a variant for it; [CommentTarget::iter] finds
/// it, and its content check subject goes in [content_check]'s subjects.
/// @since 261021
#[enum_dispatch(CommentTargetAdapter)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter)]
pub enum CommentTarget {
    Song(song::SongComments),
    /// @since 261024
    Post(post::PostComments),
}

/// What every kind of comment target provides, implemented once per kind in its own file.
// Only called through the concrete CommentTarget, so the futures' auto traits (Send) are known
#[allow(async_fn_in_trait)]
#[enum_dispatch]
pub trait CommentTargetAdapter {
    /// The kind's name in the API and in storage, such as `song`.
    fn name(&self) -> &'static str;

    /// The content check subject comments on this kind are checked as.
    fn subject(&self) -> &'static str;

    /// The targets of these ids that exist, by id.
    async fn load_many(&self, pool: &PgPool, target_ids: &[i64]) -> sqlx::Result<HashMap<i64, TargetInfo>>;

    /// Where a notification about comment `comment_id` takes its reader, given the preview of
    /// what the comment is on. None if `on` isn't this kind's.
    fn view_intent(&self, on: &UgcPreview, comment_id: CommentId) -> Option<ContentIntent>;
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
        Self::iter().find(|x| x.name() == s)
    }

    pub fn as_str(self) -> &'static str {
        self.name()
    }

    pub async fn load(self, pool: &PgPool, target_id: i64) -> sqlx::Result<Option<TargetInfo>> {
        Ok(self.load_many(pool, &[target_id]).await?.remove(&target_id))
    }

    /// Where a notification about comment `comment_id` takes its reader, asking each kind
    /// whether `on` is its own.
    pub fn view_intent_of(on: &UgcPreview, comment_id: CommentId) -> Option<ContentIntent> {
        Self::iter().find_map(|x| x.view_intent(on, comment_id))
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
    /// Visible replies.
    pub reply_count: i64,
}

/// Hydrates comments into what `viewer_uid` sees: looks up their authors, whom they reply to and
/// whether the viewer liked them, a query each for the whole batch, and blanks removed ones.
pub(crate) async fn hydrate(pool: &PgPool, comments: Vec<Comment>, viewer_uid: Option<UserId>) -> sqlx::Result<Vec<CommentItem>> {
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
        let removed = !x.is_visible();
        CommentItem {
            id: x.id,
            target_type: x.target_type,
            target_id: x.target_id,
            root_id: x.root_id,
            reply_to_id: x.reply_to_id,
            reply_to_user: reply_to_uids.get(&x.id).and_then(|uid| users.get(uid)).cloned(),
            author: if removed { None } else { users.get(&x.user_id).cloned() },
            content: if removed { String::new() } else { x.content },
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
    /// Visible comments and replies on the target.
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
    let roots = CommentDao::page_roots(pool, target.as_str(), target_id, by_likes, page_size, page_index * page_size).await?;
    let root_ids: Vec<CommentId> = roots.iter().map(|x| x.id).collect();
    let replies = CommentDao::list_first_replies(pool, &root_ids, FIRST_REPLIES).await?;
    let reply_counts: HashMap<CommentId, i64> = roots.iter().map(|x| (x.id, x.visible_reply_count as i64)).collect();
    let total = CommentDao::count_visible_by_target(pool, target.as_str(), target_id).await?;

    let mut replies_by_root: HashMap<CommentId, Vec<CommentItem>> = HashMap::new();
    for reply in hydrate(pool, replies, viewer_uid).await? {
        if let Some(root_id) = reply.root_id {
            replies_by_root.entry(root_id).or_default().push(reply);
        }
    }
    let threads = hydrate(pool, roots, viewer_uid).await?.into_iter().map(|comment| CommentThread {
        replies: replies_by_root.remove(&comment.id).unwrap_or_default(),
        reply_count: reply_counts.get(&comment.id).copied().unwrap_or(0),
        comment,
    }).collect();
    Ok(CommentPage { total, threads })
}

pub struct ReplyPage {
    /// Visible replies under the root.
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
    let replies = CommentDao::page_replies(pool, root_id, page_size, page_index * page_size).await?;
    Ok(ReplyPage { total: root.visible_reply_count as i64, replies: hydrate(pool, replies, viewer_uid).await? })
}

/// Posts a comment on a target, or a reply to `reply_to_id`. Checked first: blocked ones aren't
/// saved, and ones that can't be checked now wait to be checked again, seen by no one until they
/// pass, and return `None`.
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
) -> ServiceResult<Option<CommentItem>, CommentError> {
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
                .filter(|x| x.is_visible() && x.target_type == target.as_str() && x.target_id == target_id);
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
    let subjects: Vec<&str> = CommentTarget::iter().map(|x| x.subject()).collect();
    let mut count = CommentDao::count_by_user_after(pool, uid, since).await?
        + ContentReviewDao::count_pending_by_actor_after(pool, uid, &subjects, since).await?;
    if checker.is_some() {
        count += ContentCheckDao::count_blocked_by_actor_and_subject_since(pool, uid, target.subject(), since).await?;
    }
    if count >= COMMENTS_PER_HOUR {
        return business(CommentError::RateLimited);
    }

    let pending = PendingComment {
        id: CommentDao::next_id(pool).await?,
        target_type: target.as_str().to_string(),
        target_id,
        user_id: uid,
        root_id,
        reply_to_id,
        reply_to_uid,
        content: content.to_string(),
        create_time: now,
    };
    let verdict = match checker {
        Some(checker) => checker.try_check_text(pool, target.subject(), uid, pending.id.0, content).await?,
        None => Some(Verdict::Pass),
    };
    let mut tx = pool.begin().await?;
    let saved = match verdict {
        Some(Verdict::Pass) => Some(save(&mut tx, &pending).await?),
        Some(Verdict::Blocked(_)) => return business(CommentError::Rejected),
        None => {
            let payload = serde_json::to_value(&pending)?;
            content_review::enqueue(&mut tx, target.subject(), pending.id.0, uid, payload, now).await?;
            None
        }
    };
    tx.commit().await?;
    Ok(match saved {
        Some(comment) => Some(hydrate(pool, vec![comment], Some(uid)).await?.remove(0)),
        None => None,
    })
}

/// A comment waiting for a content check, kept in its review's payload until it passes.
///
/// Old payloads stay in the queue across deploys, and a rollback or an older instance still
/// running can read newer ones. So a field added later must have `#[serde(default)]` or be an
/// `Option`, for payloads written before it; and must be safe to lose, as older code ignores it.
/// Never add `#[serde(deny_unknown_fields)]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingComment {
    pub id: CommentId,
    pub target_type: String,
    pub target_id: i64,
    pub user_id: UserId,
    pub root_id: Option<CommentId>,
    pub reply_to_id: Option<CommentId>,
    pub reply_to_uid: UserId,
    pub content: String,
    /// When it was posted, which it keeps once saved.
    pub create_time: DateTime<Utc>,
}

/// Saves a comment that passed its content check, or was published without one, and counts it
/// on its root.
async fn save(tx: &mut sqlx::PgTransaction<'_>, pending: &PendingComment) -> sqlx::Result<Comment> {
    let comment = Comment {
        id: pending.id,
        target_type: pending.target_type.clone(),
        target_id: pending.target_id,
        user_id: pending.user_id,
        root_id: pending.root_id,
        reply_to_id: pending.reply_to_id,
        reply_to_uid: pending.reply_to_uid,
        content: pending.content.clone(),
        is_hidden: false,
        like_count: 0,
        visible_reply_count: 0,
        delete_time: None,
        create_time: pending.create_time,
        update_time: Utc::now(),
    };
    CommentDao::insert(&mut **tx, &comment).await?;
    if let Some(root_id) = comment.root_id {
        CommentDao::add_visible_reply_count(&mut **tx, root_id, 1).await?;
    }
    Ok(comment)
}

/// Saves a comment that waited for its content check, once it passes, in `tx`, the transaction
/// that ends its review.
pub async fn publish_pending(tx: &mut sqlx::PgTransaction<'_>, pending: &PendingComment) -> sqlx::Result<()> {
    save(tx, pending).await.map(|_| ())
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
    let mut tx = pool.begin().await?;
    change_visibility(&mut tx, comment_id, |x| x.delete_time = Some(Utc::now())).await?;
    tx.commit().await?;
    Ok(())
}

/// Likes a public comment. Liking it again does nothing.
pub async fn like(pool: &PgPool, uid: UserId, comment_id: CommentId) -> ServiceResult<(), CommentError> {
    if !CommentDao::get_by_id(pool, comment_id).await?.is_some_and(|x| x.is_visible()) {
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

/// Hides or shows a comment, for report decisions.
pub async fn set_hidden(tx: &mut sqlx::PgTransaction<'_>, comment_id: CommentId, hidden: bool) -> sqlx::Result<()> {
    change_visibility(tx, comment_id, |x| x.is_hidden = hidden).await
}

/// Changes what decides whether a comment is visible, through `change`, and keeps its root's
/// derived [Comment::visible_reply_count] in step in the same transaction. Every change to the
/// `is_hidden` or `delete_time` of a saved comment goes through here. Does nothing if
/// there's no such comment.
async fn change_visibility(tx: &mut sqlx::PgTransaction<'_>, comment_id: CommentId, change: impl FnOnce(&mut Comment)) -> sqlx::Result<()> {
    // Locked, so two changes at once can't both count the same change
    let Some(before) = CommentDao::get_by_id_for_update(&mut **tx, comment_id).await? else {
        return Ok(());
    };
    let mut after = before.clone();
    change(&mut after);
    after.update_time = Utc::now();
    CommentDao::update_visibility(&mut **tx, &after).await?;
    if let Some(root_id) = after.root_id && before.is_visible() != after.is_visible() {
        CommentDao::add_visible_reply_count(&mut **tx, root_id, if after.is_visible() { 1 } else { -1 }).await?;
    }
    Ok(())
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

/// How many visible comments replying to `uid` or on what they own others posted after `after`, up
/// to [MAX_UNREAD_REPLIES].
pub async fn count_received_replies_after(pool: &PgPool, uid: UserId, after: DateTime<Utc>) -> sqlx::Result<i64> {
    CommentDao::count_replies_to_after(pool, uid, after, MAX_UNREAD_REPLIES).await
}

/// Comments replying to `uid` or on what they own, by others, newest first. Ones on targets `uid`
/// can't see are left out. Returns the page and whether there are more.
pub async fn received_replies(
    pool: &PgPool,
    uid: UserId,
    before: Option<(DateTime<Utc>, CommentId)>,
    limit: i64,
) -> sqlx::Result<(Vec<ReceivedReplyItem>, bool)> {
    let mut comments = CommentDao::list_replies_to(pool, uid, before, limit + 1).await?;
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

    let items = hydrate(pool, comments, Some(uid)).await?
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
