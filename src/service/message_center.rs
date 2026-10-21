//! The message center: unread counts of every kind of message, and the kinds that are read as a
//! whole, received likes, new followers and replies. Each of those keeps one read time per user;
//! opening it marks everything up to now read. System notifications are read one by one, see
//! [crate::service::notification].

use crate::db::comment::{CommentDao, CommentId};
use crate::db::follow::FollowDao;
use crate::db::message_read_mark::{MessageReadMark, MessageReadMarkDao};
use crate::db::received_like::ReceivedLikeDao;
use crate::db::song::{ISongDao, Song, SongDao, SongId};
use crate::db::user::{IUserDao, UserDao, UserId};
use crate::service::comment::{self, CommentItem, CommentTarget};
use crate::service::notification;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;

/// Before a user first opens a kind, only messages from this many days back are unread, so a
/// long-time user doesn't start with thousands.
const DEFAULT_UNREAD_DAYS: i64 = 30;
/// Likers shown per song.
const LATEST_LIKERS: i64 = 3;

/// A kind of message read as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Like,
    Follow,
    /// Replies to the user's comments, and comments on their songs.
    /// @since 261021
    Reply,
}

impl Channel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "like" => Some(Self::Like),
            "follow" => Some(Self::Follow),
            "reply" => Some(Self::Reply),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Like => "like",
            Self::Follow => "follow",
            Self::Reply => "reply",
        }
    }
}

/// @since 261006
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub system_unread: i64,
    pub like_unread: i64,
    pub follow_unread: i64,
    /// @since 261021
    pub reply_unread: i64,
    /// Likes after this are unread.
    pub like_read_time: DateTime<Utc>,
    /// Followers who followed after this are new.
    pub follow_read_time: DateTime<Utc>,
    /// Replies after this are unread.
    /// @since 261021
    pub reply_read_time: DateTime<Utc>,
}

pub async fn summary(pool: &PgPool, uid: UserId) -> sqlx::Result<Summary> {
    let marks: HashMap<String, DateTime<Utc>> = MessageReadMarkDao::list_by_uid(pool, uid).await?
        .into_iter()
        .map(|x| (x.channel, x.read_time))
        .collect();
    let default_read_time = Utc::now() - Duration::days(DEFAULT_UNREAD_DAYS);
    let read_time = |channel: Channel| marks.get(channel.as_str()).copied().unwrap_or(default_read_time);
    let like_read_time = read_time(Channel::Like);
    let follow_read_time = read_time(Channel::Follow);
    let reply_read_time = read_time(Channel::Reply);

    Ok(Summary {
        system_unread: notification::unread_count(pool, uid).await?,
        like_unread: ReceivedLikeDao::count_after(pool, uid, like_read_time).await?,
        follow_unread: FollowDao::count_followers_after(pool, uid, follow_read_time).await?,
        reply_unread: CommentDao::count_published_replies_to_after(pool, uid, reply_read_time).await?,
        like_read_time,
        follow_read_time,
        reply_read_time,
    })
}

/// Marks everything in `channel` up to now read. Returns the new read time.
pub async fn mark_read(pool: &PgPool, uid: UserId, channel: Channel) -> sqlx::Result<DateTime<Utc>> {
    // PostgreSQL keeps microseconds; return what is stored
    let now = Utc::now().trunc_subsecs(6);
    let mark = MessageReadMark {
        uid,
        channel: channel.as_str().to_string(),
        read_time: now,
        update_time: now,
    };
    if MessageReadMarkDao::update(pool, &mark).await? {
        return Ok(now);
    }
    match MessageReadMarkDao::insert(pool, &mark).await {
        Ok(()) => Ok(now),
        // Another request inserted it first
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            MessageReadMarkDao::update(pool, &mark).await?;
            Ok(now)
        }
        Err(e) => Err(e),
    }
}

/// @since 261006
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Liker {
    pub uid: UserId,
    pub username: String,
    pub avatar_url: Option<String>,
}

/// The likes one song received from others.
/// @since 261006
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedLikeItem {
    pub song_id: SongId,
    pub song_display_id: String,
    pub song_title: String,
    pub cover_url: String,
    /// All likes from others, not only unread ones.
    pub like_count: i64,
    pub latest_like_time: DateTime<Utc>,
    /// The latest few, newest first.
    pub latest_likers: Vec<Liker>,
}

/// Songs of `uid` with likes from others, most recently liked first. Returns the page and whether
/// there are more.
pub async fn received_likes(
    pool: &PgPool,
    uid: UserId,
    before: Option<(DateTime<Utc>, SongId)>,
    limit: i64,
) -> sqlx::Result<(Vec<ReceivedLikeItem>, bool)> {
    let mut liked = ReceivedLikeDao::list_liked_songs(pool, uid, before, limit + 1).await?;
    let has_more = liked.len() as i64 > limit;
    liked.truncate(limit as usize);

    let song_ids: Vec<SongId> = liked.iter().map(|x| x.song_id).collect();
    let mut songs: HashMap<SongId, Song> = SongDao::list_by_ids(pool, &song_ids).await?
        .into_iter()
        .map(|x| (x.id, x))
        .collect();
    let latest = ReceivedLikeDao::list_latest_by_songs(pool, &song_ids, LATEST_LIKERS).await?;
    let user_ids: Vec<UserId> = latest.iter().map(|x| x.user_id).collect();
    let users: HashMap<UserId, Liker> = UserDao::list_by_ids(pool, &user_ids).await?
        .into_iter()
        .map(|x| (x.id, Liker { uid: x.id, username: x.username, avatar_url: x.avatar_url }))
        .collect();
    let mut likers: HashMap<SongId, Vec<Liker>> = HashMap::new();
    for like in latest {
        if let Some(user) = users.get(&like.user_id) {
            likers.entry(like.song_id).or_default().push(user.clone());
        }
    }

    let items = liked.into_iter().filter_map(|x| {
        let song = songs.remove(&x.song_id)?;
        Some(ReceivedLikeItem {
            latest_likers: likers.remove(&x.song_id).unwrap_or_default(),
            song_id: x.song_id,
            song_display_id: song.display_id,
            song_title: song.title,
            cover_url: song.cover_art_url,
            like_count: x.like_count,
            latest_like_time: x.latest_like_time,
        })
    }).collect();
    Ok((items, has_more))
}

/// What a received reply is on.
/// @since 261021
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyTarget {
    /// Such as `song`.
    pub target_type: String,
    pub target_id: i64,
    /// The song's title.
    pub title: String,
    pub cover_url: Option<String>,
    /// The song's display id, such as `JM-ABCD-123`.
    pub display_id: String,
}

/// A comment replying to the user, or on their song.
/// @since 261021
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedReplyItem {
    pub comment: CommentItem,
    pub target: ReplyTarget,
    /// The user's comment it replies to; `None` for a comment on their song, or if that comment was
    /// deleted.
    pub replied_content: Option<String>,
}

/// Comments replying to `uid` or on their songs, by others, newest first. Ones on targets `uid`
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

    let song_ids: Vec<SongId> = comments.iter()
        .filter(|x| CommentTarget::parse(&x.target_type) == Some(CommentTarget::Song))
        .map(|x| SongId(x.target_id))
        .collect();
    let songs: HashMap<SongId, Song> = SongDao::list_by_ids(pool, &song_ids).await?
        .into_iter()
        .filter(|x| !x.is_hidden || x.uploader_uid == uid)
        .map(|x| (x.id, x))
        .collect();
    let replied_ids: Vec<CommentId> = comments.iter().filter_map(|x| x.reply_to_id).collect();
    let replied: HashMap<CommentId, String> = CommentDao::list_by_ids(pool, &replied_ids).await?
        .into_iter()
        .filter(|x| x.delete_time.is_none())
        .map(|x| (x.id, x.content))
        .collect();

    let items = comment::to_items(pool, comments, Some(uid)).await?
        .into_iter()
        .filter_map(|comment| {
            let song = songs.get(&SongId(comment.target_id))?;
            Some(ReceivedReplyItem {
                target: ReplyTarget {
                    target_type: comment.target_type.clone(),
                    target_id: comment.target_id,
                    title: song.title.clone(),
                    cover_url: Some(song.cover_art_url.clone()),
                    display_id: song.display_id.clone(),
                },
                replied_content: comment.reply_to_id.and_then(|x| replied.get(&x).cloned()),
                comment,
            })
        })
        .collect();
    Ok((items, has_more))
}
