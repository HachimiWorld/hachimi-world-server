use crate::db::user::{IUserDao, UserDao, UserId};
use crate::service::connection_account;
use crate::service::connection_account::ConnectionAccount;
use itertools::Itertools;
use redis::aio::ConnectionManager;
use redis::{AsyncTypedCommands, MSetOptions, SetExpiry};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;

/// Shared user profile model — returned by all APIs that include user info.
/// `is_following` / `is_followed_by` are `None` unless enriched via `get_user_profile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicUserProfile {
    pub uid: UserId,
    pub username: String,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub gender: Option<i32>,
    pub is_banned: bool,
    /// @since 260402
    pub connected_accounts: Vec<ConnectedAccountItem>,
    /// @since 260619
    pub follower_count: i64,
    /// @since 260619
    pub following_count: i64,
    /// @since 260619 — None when viewing own profile or unauthenticated, or when caller uses `get_common_user_profile`
    pub is_following: Option<bool>,
    /// @since 260619 — None when viewing own profile or unauthenticated, or when caller uses `get_common_user_profile`
    pub is_followed_by: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedAccountItem {
    pub r#type: String,
    pub id: String,
    pub name: String,
}

/// Get user profiles from DB + Redis cache.  Follow-relationship fields (`is_following`,
/// `is_followed_by`) are left `None` unless `viewer_uid` is provided.
pub async fn get_public_profile(
    redis: ConnectionManager,
    sql_pool: &PgPool,
    user_ids: &[UserId],
    viewer_uid: Option<UserId>,
) -> anyhow::Result<HashMap<UserId, PublicUserProfile>> {
    let mut profiles = get_common_user_profile(redis, sql_pool, user_ids).await?;

    if let Some(vid) = viewer_uid {
        for (&uid, profile) in profiles.iter_mut() {
            if uid == vid {
                continue;
            }
            let (is_following, is_followed_by) =
                crate::service::follow::check_follow_relationship(sql_pool, vid, uid).await?;
            profile.is_following = Some(is_following);
            profile.is_followed_by = Some(is_followed_by);
        }
    }

    Ok(profiles)
}

/// Internal: fetch profiles from DB + Redis cache, without enrichments.
pub(crate) async fn get_common_user_profile(
    redis: ConnectionManager,
    sql_pool: &PgPool,
    user_ids: &[UserId],
) -> anyhow::Result<HashMap<UserId, PublicUserProfile>> {
    if user_ids.is_empty() { 
        return Ok(HashMap::new());
    }
    
    let unique_uids = user_ids.iter().copied().unique().collect_vec();
    let mut cached_profiles = get_from_cache(redis.clone(), &unique_uids).await?;

    let missed_ids = unique_uids.into_iter().filter(|uid| !cached_profiles.contains_key(uid)).collect_vec();
    if missed_ids.is_empty() {
        return Ok(cached_profiles);
    }

    let users = UserDao::list_by_ids(sql_pool, &missed_ids).await?;

    // parallel get connections for each user and fill in the profile, but for now just return empty connections
    let connections: HashMap<UserId, Vec<ConnectionAccount>> = futures::future::join_all(users.iter().map(|u| {
        connection_account::list_connections(sql_pool, redis.clone(), u.id, true)
    })).await.into_iter()
        .zip(users.iter())
        .map(|(connections_result, user)| {
            let connections = connections_result.unwrap_or_default();
            (user.id, connections)
        })
        .collect();

    let profiles: HashMap<_, _> = users.into_iter()
        .map(|u| PublicUserProfile {
            uid: u.id,
            username: u.username,
            avatar_url: u.avatar_url,
            bio: u.bio,
            gender: u.gender,
            is_banned: u.is_banned,
            connected_accounts: connections.get(&u.id).cloned().unwrap_or_default().into_iter().map(|c| ConnectedAccountItem {
                r#type: c.r#type,
                id: c.id,
                name: c.name
            }).collect_vec(),
            follower_count: u.follower_count.unwrap_or(0),
            following_count: u.following_count.unwrap_or(0),
            is_following: None,
            is_followed_by: None,
        })
        .into_iter()
        .map(|x| (x.uid, x))
        .collect();
    save_to_cache(redis, &profiles).await?;
    cached_profiles.extend(profiles);
    Ok(cached_profiles)
}

fn gen_cache_key(uid: UserId) -> String {
    format!("user_profile:uid={}", uid)
}

async fn get_from_cache(mut redis: ConnectionManager, user_ids: &[UserId]) -> anyhow::Result<HashMap<UserId, PublicUserProfile>> {
    if user_ids.is_empty() { 
        return Ok(HashMap::new());
    }
    // mget
    let keys: Vec<String> = user_ids.iter().map(|uid| gen_cache_key(*uid)).collect();
    let values: Vec<Option<String>> = redis.mget(&keys).await?;
    let result: HashMap<UserId, PublicUserProfile> = user_ids.iter().cloned().zip(values.into_iter())
        .filter_map(|(uid, value)| {
            value.and_then(|v| serde_json::from_str::<PublicUserProfile>(&v).ok()).map(|profile| (uid, profile))
        })
        .collect();
    Ok(result)
}

async fn save_to_cache(mut redis: ConnectionManager, profiles: &HashMap<UserId, PublicUserProfile>) -> anyhow::Result<()> {
    if profiles.is_empty() { 
        return Ok(());
    }
    
    // mset
    let cache_key_value_pairs: Vec<(String, String)> = profiles.iter()
        .filter_map(
            |(uid, profile)| serde_json::to_string(profile).ok().map(|value|
                (gen_cache_key(*uid), value)
            ))
        .collect();
    if !cache_key_value_pairs.is_empty() {
        redis.mset_ex(&cache_key_value_pairs, MSetOptions::default().with_expiration(SetExpiry::EX(3000))).await?;
    }
    Ok(())
}
/// A random username such as `神人01234567`, as given at registration.
/// Widest a username may be, where a CJK character counts as 2 and most others as 1: 10 Chinese
/// characters or 20 Latin letters take about the same space on screen.
/// @since 261017
pub const USERNAME_MAX_WIDTH: usize = 20;
/// A cap on characters as well, since zero-width ones add no width.
/// @since 261017
pub const USERNAME_MAX_CHARS: usize = 32;

/// Why `name` can't be a username, or `None` if it can. Width follows Unicode East Asian Width
/// (UAX #11): wide and fullwidth characters, such as CJK and most emoji, count as 2, ambiguous ones
/// as 1, and zero-width ones as 0.
/// @since 261017
pub fn username_problem(name: &str) -> Option<String> {
    use unicode_width::UnicodeWidthStr;
    if name.chars().any(char::is_control) {
        return Some("Username cannot contain control characters such as line breaks".to_string());
    }
    if name.starts_with(char::is_whitespace) || name.ends_with(char::is_whitespace) {
        return Some("Username cannot start or end with a space".to_string());
    }
    if let Some(c) = invisible_char(name) {
        return Some(format!("Username cannot contain invisible characters such as U+{:04X}", c as u32));
    }
    let width = name.width();
    if width == 0 {
        return Some("Username cannot be empty".to_string());
    }
    if width > USERNAME_MAX_WIDTH || name.chars().count() > USERNAME_MAX_CHARS {
        return Some(format!(
            "Username must be at most {USERNAME_MAX_WIDTH} wide, where a CJK character counts as 2"
        ));
    }
    None
}

/// The first character in `name` that shows nothing by itself, so that two names could look the
/// same or an @mention could miss: zero-width characters, bidirectional text controls and the rest
/// of Default_Ignorable_Code_Point (UAX #44), and the blank Braille pattern. Emoji keep what their
/// sequences need: a ZWJ between emoji, a variation selector after an emoji, and tags after 🏴.
/// @since 261027
fn invisible_char(name: &str) -> Option<char> {
    use icu_properties::CodePointSetData;
    use icu_properties::props::{DefaultIgnorableCodePoint, Emoji, EmojiModifier, ExtendedPictographic};
    let ignorable = CodePointSetData::new::<DefaultIgnorableCodePoint>();
    let emoji = CodePointSetData::new::<Emoji>();
    let pictographic = CodePointSetData::new::<ExtendedPictographic>();
    let modifier = CodePointSetData::new::<EmojiModifier>();
    let chars: Vec<char> = name.chars().collect();
    chars.iter().enumerate().find_map(|(i, &c)| {
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        let allowed = match c {
            '\u{2800}' => false,
            // Digits, # and * are emoji only as keycaps, followed by U+20E3
            '\u{FE0E}' | '\u{FE0F}' => prev.is_some_and(|p| emoji.contains(p) && (!p.is_ascii() || next == Some('\u{20E3}'))),
            '\u{200D}' => prev.is_some_and(|p| pictographic.contains(p) || modifier.contains(p) || p == '\u{FE0F}')
                && next.is_some_and(|n| pictographic.contains(n)),
            '\u{E0020}'..='\u{E007F}' => prev.is_some_and(|p| p == '\u{1F3F4}' || ('\u{E0020}'..='\u{E007E}').contains(&p)),
            _ => !ignorable.contains(c),
        };
        (!allowed).then_some(c)
    })
}

pub fn generate_username() -> String {
    use rand::RngExt;
    format!("神人{:08}", rand::rng().random_range(0..100000000))
}

/// A part of a profile the platform can reset.
/// @since 261008
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileField {
    Avatar,
    Bio,
    Username,
}

/// Resets `fields` of a user's profile in the caller's transaction: the avatar and bio are cleared
/// and the username is replaced with a random one. Returns the updated user if they exist. Call
/// [refresh_after_profile_change] after committing.
/// @since 261008
pub async fn reset_profile(
    tx: &mut sqlx::PgTransaction<'_>,
    uid: UserId,
    fields: &[ProfileField],
) -> anyhow::Result<Option<crate::db::user::User>> {
    use crate::db::CrudDao;
    let Some(mut user) = UserDao::get_by_id(&mut **tx, uid).await? else {
        return Ok(None);
    };
    for field in fields {
        match field {
            ProfileField::Avatar => user.avatar_url = None,
            ProfileField::Bio => user.bio = None,
            ProfileField::Username => {
                let mut name = generate_username();
                while UserDao::get_by_username(&mut **tx, &name).await?.is_some() {
                    name = generate_username();
                }
                user.username = name;
            }
        }
    }
    user.update_time = chrono::Utc::now();
    UserDao::update_by_id(&mut **tx, &user).await?;
    Ok(Some(user))
}

/// Updates the search index and clears the cached profile after a change to the user is saved, such
/// as an edit by the user or [reset_profile]. Failures are logged.
/// @since 261008
pub async fn refresh_after_profile_change(state: &crate::web::state::AppState, user: &crate::db::user::User) {
    if let Err(e) = crate::search::user::update_user_document(&state.meilisearch, crate::search::user::UserDocument {
        id: user.id,
        avatar_url: user.avatar_url.clone(),
        name: user.username.clone(),
        follower_count: user.follower_count.unwrap_or(0),
    }).await {
        tracing::warn!(uid = user.id.0, "Failed to update the user search index: {e:?}");
    }
    let mut redis = state.redis_conn.clone();
    if let Err(e) = redis.del(gen_cache_key(user.id)).await {
        tracing::warn!(uid = user.id.0, "Failed to clear the cached profile: {e:?}");
    }
}

#[cfg(test)]
mod username_tests {
    use super::{generate_username, username_problem};

    #[test]
    fn counts_cjk_as_two() {
        assert_eq!(username_problem("哈基米"), None);
        assert_eq!(username_problem("一二三四五六七八九十"), None); // 10 CJK = 20
        assert!(username_problem("一二三四五六七八九十一").is_some()); // 11 CJK = 22
        assert_eq!(username_problem("abcdefghijklmnopqrst"), None); // 20 Latin
        assert!(username_problem("abcdefghijklmnopqrstu").is_some()); // 21 Latin
        assert_eq!(username_problem("小风xiaofeng"), None); // 4 + 8
        assert_eq!(username_problem("罗兹official"), None);
        assert_eq!(username_problem("(｡•̀ᴗ-)✧"), None);
        assert_eq!(username_problem(&generate_username()), None);
    }

    #[test]
    fn rejects_empty_invisible_and_control() {
        assert!(username_problem("").is_some());
        assert!(username_problem("\u{200B}\u{200B}").is_some()); // zero-width spaces only
        assert!(username_problem("名字\n第二行").is_some());
        assert!(username_problem("tab\there").is_some());
        // Zero-width characters add no width, so the character cap stops padding with them
        assert!(username_problem(&format!("a{}", "\u{200B}".repeat(40))).is_some());
    }

    #[test]
    fn rejects_hidden_characters_inside_names() {
        assert!(username_problem("哈基\u{200B}米").is_some()); // zero-width space
        assert!(username_problem("哈基\u{200D}米").is_some()); // ZWJ between letters
        assert!(username_problem("哈基\u{2060}米").is_some()); // word joiner
        assert!(username_problem("\u{FEFF}哈基米").is_some()); // BOM
        assert!(username_problem("abc\u{202E}fed").is_some()); // right-to-left override
        assert!(username_problem("abc\u{2066}def\u{2069}").is_some()); // bidi isolate
        assert!(username_problem("哈基米\u{00AD}").is_some()); // soft hyphen
        assert!(username_problem("哈\u{3164}米").is_some()); // Hangul filler
        assert!(username_problem("哈\u{2800}米").is_some()); // blank Braille pattern
        assert!(username_problem("Alice\u{FE0F}").is_some()); // variation selector on a letter
        assert!(username_problem("Alice1\u{FE0F}").is_some()); // on a digit, not a keycap
    }

    #[test]
    fn rejects_surrounding_spaces() {
        assert!(username_problem(" 哈基米").is_some());
        assert!(username_problem("哈基米 ").is_some());
        assert!(username_problem("哈基米\u{3000}").is_some()); // ideographic space
        assert!(username_problem("\u{00A0}哈基米").is_some()); // no-break space
        assert!(username_problem("   ").is_some());
    }

    #[test]
    fn keeps_spaces_punctuation_and_emoji() {
        assert_eq!(username_problem("Hachimi World"), None);
        assert_eq!(username_problem("哈基米 の 歌"), None);
        assert_eq!(username_problem("Mr.O'Neil-Jr_"), None);
        assert_eq!(username_problem("哈基米❤️"), None); // variation selector after an emoji
        assert_eq!(username_problem("❤️‍🔥哈基米"), None); // ZWJ sequence
        assert_eq!(username_problem("👨🏽‍💻coder"), None); // skin tone then ZWJ
        assert_eq!(username_problem("🏳️‍🌈"), None);
        assert_eq!(username_problem("1️⃣号"), None); // keycap
        assert_eq!(username_problem("🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}"), None); // England flag
    }
}
