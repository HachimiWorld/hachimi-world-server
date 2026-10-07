//! The message center: unread counts of every kind of message, and the kinds that are read as a
//! whole, received likes and new followers. Each of those keeps one read time per user; opening it
//! marks everything up to now read. System notifications are read one by one, see
//! [crate::service::notification].

use crate::db::follow::FollowDao;
use crate::db::message_read_mark::{MessageReadMark, MessageReadMarkDao};
use crate::db::received_like::ReceivedLikeDao;
use crate::db::song::{ISongDao, Song, SongDao};
use crate::db::user::{IUserDao, UserDao};
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
}

impl Channel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "like" => Some(Self::Like),
            "follow" => Some(Self::Follow),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Like => "like",
            Self::Follow => "follow",
        }
    }
}

/// @since 261006
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub system_unread: i64,
    pub like_unread: i64,
    pub follow_unread: i64,
    /// Likes after this are unread.
    pub like_read_time: DateTime<Utc>,
    /// Followers who followed after this are new.
    pub follow_read_time: DateTime<Utc>,
}

pub async fn summary(pool: &PgPool, uid: i64) -> sqlx::Result<Summary> {
    let marks: HashMap<String, DateTime<Utc>> = MessageReadMarkDao::list_by_uid(pool, uid).await?
        .into_iter()
        .map(|x| (x.channel, x.read_time))
        .collect();
    let default_read_time = Utc::now() - Duration::days(DEFAULT_UNREAD_DAYS);
    let read_time = |channel: Channel| marks.get(channel.as_str()).copied().unwrap_or(default_read_time);
    let like_read_time = read_time(Channel::Like);
    let follow_read_time = read_time(Channel::Follow);

    Ok(Summary {
        system_unread: notification::unread_count(pool, uid).await?,
        like_unread: ReceivedLikeDao::count_after(pool, uid, like_read_time).await?,
        follow_unread: FollowDao::count_followers_after(pool, uid, follow_read_time).await?,
        like_read_time,
        follow_read_time,
    })
}

/// Marks everything in `channel` up to now read. Returns the new read time.
pub async fn mark_read(pool: &PgPool, uid: i64, channel: Channel) -> sqlx::Result<DateTime<Utc>> {
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
    pub uid: i64,
    pub username: String,
    pub avatar_url: Option<String>,
}

/// The likes one song received from others.
/// @since 261006
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedLikeItem {
    pub song_id: i64,
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
    uid: i64,
    before: Option<(DateTime<Utc>, i64)>,
    limit: i64,
) -> sqlx::Result<(Vec<ReceivedLikeItem>, bool)> {
    let mut liked = ReceivedLikeDao::list_liked_songs(pool, uid, before, limit + 1).await?;
    let has_more = liked.len() as i64 > limit;
    liked.truncate(limit as usize);

    let song_ids: Vec<i64> = liked.iter().map(|x| x.song_id).collect();
    let mut songs: HashMap<i64, Song> = SongDao::list_by_ids(pool, &song_ids).await?
        .into_iter()
        .map(|x| (x.id, x))
        .collect();
    let latest = ReceivedLikeDao::list_latest_by_songs(pool, &song_ids, LATEST_LIKERS).await?;
    let user_ids: Vec<i64> = latest.iter().map(|x| x.user_id).collect();
    let users: HashMap<i64, Liker> = UserDao::list_by_ids(pool, &user_ids).await?
        .into_iter()
        .map(|x| (x.id, Liker { uid: x.id, username: x.username, avatar_url: x.avatar_url }))
        .collect();
    let mut likers: HashMap<i64, Vec<Liker>> = HashMap::new();
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
