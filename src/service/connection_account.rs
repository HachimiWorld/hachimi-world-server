use crate::db::user_connection_accounts::{IUserConnectionAccountDao, UserConnectionAccount, UserConnectionAccountDao};
use crate::util::bilibili;
use crate::util::redlock::RedLock;
use anyhow::{anyhow, bail, Context};
use chrono::Utc;
use itertools::Itertools;
use redis::aio::ConnectionManager;
use redis::AsyncTypedCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::warn;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionAccount {
    pub r#type: String,
    pub id: String,
    pub name: String,
    pub public: bool,
}

pub async fn list_connections(sql: &PgPool, mut redis: ConnectionManager, uid: i64, public: bool) -> anyhow::Result<Vec<ConnectionAccount>> {
    let cache = get_connections_from_cache(&mut redis, uid, public).await?;
    if let Some(cached) = cache {
        return Ok(cached);
    }

    let connections = if public {
        UserConnectionAccountDao::list_public_by_user_id(sql, uid).await?
    } else {
        UserConnectionAccountDao::list_by_user_id(sql, uid).await?
    };

    let mapped = connections.into_iter().map(|c| ConnectionAccount {
        r#type: c.provider_type,
        id: c.provider_account_id,
        name: c.provider_account_name,
        public: c.public,
    }).collect_vec();

    save_connections_to_cache(&mut redis, uid, public, &mapped).await?;
    Ok(mapped)
}

async fn get_connections_from_cache(redis: &mut ConnectionManager, uid: i64, public: bool) -> anyhow::Result<Option<Vec<ConnectionAccount>>> {
    let cache_key = format!("user_account_connections:uid={},public={}", uid, public);
    if let Some(cached) = redis.get(&cache_key).await? &&
        let Ok(parsed) = serde_json::from_str::<Vec<ConnectionAccount>>(&cached)
    {
        Ok(Some(parsed))
    } else {
        Ok(None)
    }
}

async fn save_connections_to_cache(redis: &mut ConnectionManager, uid: i64, public: bool, connections: &[ConnectionAccount]) -> anyhow::Result<()> {
    let cache_key = format!("user_account_connections:uid={},public={}", uid, public);
    redis.set_ex(cache_key, serde_json::to_string(connections)?, 3000).await?;
    Ok(())
}

async fn invalidate_connections_cache(redis: &mut ConnectionManager, uid: i64) -> anyhow::Result<()> {
    let cache_key_public = format!("user_account_connections:uid={},public=true", uid);
    let cache_key_private = format!("user_account_connections:uid={},public=false", uid);

    redis.del(cache_key_public).await?;
    redis.del(cache_key_private).await?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct ChallengeCacheData {
    challenge_id: String,
    uid: i64,
    provider_type: String,
    provider_account_id: String,
    challenge: String,
}

pub struct Challenge {
    pub challenge_id: String,
    pub challenge: String,
}

const CHALLENGE_CHARS: [char; 13] = ['哈', '基', '米', '南', '北', '绿', '豆', '哦', '马', '自', '立', '曼', '波'];

fn random_challenge_string() -> String {
    // 8 random chars from CHALLENGE_CHARS
    let challenge: String = (0..8).map(|_| {
        let idx = rand::random::<u8>() as usize % CHALLENGE_CHARS.len();
        CHALLENGE_CHARS[idx]
    }).collect();
    challenge
}

/// Generate a challenge string for the user to verify ownership of the account on the provider side.
pub async fn generate_challenge(
    mut redis: ConnectionManager,
    uid: i64, provider_type: &str, provider_account_id: &str,
) -> anyhow::Result<Challenge> {
    let challenge = random_challenge_string();
    match provider_type {
        "bilibili" => {
            let challenge_id = Uuid::new_v4().to_string();
            let data = ChallengeCacheData {
                challenge_id: challenge_id.clone(),
                uid,
                provider_type: provider_type.to_string(),
                provider_account_id: provider_account_id.to_string(),
                challenge: challenge.clone(),
            };
            set_challenge_cache(&mut redis, &challenge_id, &data).await?;
            Ok(Challenge { challenge_id, challenge })
        }
        _ => anyhow::bail!("Unsupported provider type"),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyChallengeError {
    #[error("Challenge not found or expired")]
    ChallengeNotFound,
    #[error("Challenge does not match")]
    ChallengeMismatch,
    #[error("Unsupported provider type")]
    UnsupportedProviderType,
    #[error("Account already linked")]
    AlreadyLinked,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub async fn verify_challenge_and_link(
    sql: &PgPool, red_lock: RedLock, mut redis: ConnectionManager,
    uid: i64, challenge_id: &str,
) -> Result<(), VerifyChallengeError> {
    let mutex = red_lock.try_lock(&format!("verify_challenge_and_link:{}", uid)).await?
        .ok_or_else(|| anyhow!("Operating"))?;

    let challenge = get_challenge_cache(&mut redis, &challenge_id).await?
        .ok_or_else(|| VerifyChallengeError::ChallengeNotFound)?;

    if challenge.uid != uid {
        warn!(challenge_id=challenge.challenge_id, challenge_uid=challenge.uid, uid=uid, "Challenge uid mismatch");
        Err(VerifyChallengeError::ChallengeNotFound)?
    }

    let current = UserConnectionAccountDao::get_by_user_id(sql, uid, &challenge.provider_type).await
        .with_context(|| "Failed to check existing connection")?;

    if current.is_some() {
        Err(VerifyChallengeError::AlreadyLinked)?
    }

    let bili_int_id = challenge.provider_account_id.parse::<i64>().with_context(|| "Bad provider account id")?;
    let bili_profile = bilibili::get_user_profile(bili_int_id).await?;
    let verified = bili_profile.bio.ends_with(&challenge.challenge);
    if verified {
        link_account(sql, uid, &challenge.provider_type, &challenge.provider_account_id, &bili_profile.name, true).await?;
        invalidate_connections_cache(&mut redis, uid).await?;
        drop(mutex);
        Ok(())
    } else {
        Err(VerifyChallengeError::ChallengeMismatch)?
    }
}

pub async fn unlink(
    sql: &PgPool,
    mut redis: ConnectionManager,
    uid: i64,
    provider_type: &str
) -> anyhow::Result<()> {
    UserConnectionAccountDao::delete(sql, uid, provider_type).await?;
    invalidate_connections_cache(&mut redis, uid).await?;
    Ok(())
}

async fn link_account(
    sql: &PgPool,
    uid: i64,
    provider_type: &str,
    provider_account_id: &str,
    provider_account_name: &str,
    public: bool,
) -> anyhow::Result<()> {
    let value = UserConnectionAccount {
        user_id: uid,
        provider_type: provider_type.to_string(),
        provider_account_id: provider_account_id.to_string(),
        provider_account_name: provider_account_name.to_string(),
        public,
        create_time: Utc::now(),
        update_time: Utc::now(),
    };

    UserConnectionAccountDao::insert(sql, &value).await?;
    Ok(())
}

async fn set_challenge_cache(
    redis: &mut ConnectionManager,
    challenge_id: &str,
    value: &ChallengeCacheData,
) -> anyhow::Result<()> {
    let cache_key = generate_challenge_cache_key(challenge_id);
    redis.set_ex(cache_key, serde_json::to_string(&value)?, 1800).await?;
    Ok(())
}

async fn get_challenge_cache(
    redis: &mut ConnectionManager,
    challenge_id: &str,
) -> anyhow::Result<Option<ChallengeCacheData>> {
    let cache_key = generate_challenge_cache_key(challenge_id);
    let value = redis.get(&cache_key).await?;
    if let Some(value) = value &&
        let Ok(parsed) = serde_json::from_str::<ChallengeCacheData>(&value)
    {
        Ok(Some(parsed))
    } else {
        Ok(None)
    }
}

fn generate_challenge_cache_key(challenge_id: &str) -> String {
    format!("user_account_connections:challenge:{}", challenge_id)
}

pub async fn sync(sql: &PgPool, uid: i64, provider_type: &String) -> anyhow::Result<()>{
    match provider_type.as_str() {
        "bilibili" => {
            if let Ok(Some(connection)) = UserConnectionAccountDao::get_by_user_id(sql, uid, provider_type).await &&
                let Ok(bili_int_id) = connection.provider_account_id.parse::<i64>() {
                let bili_profile = bilibili::get_user_profile(bili_int_id).await?;
                if bili_profile.name != connection.provider_account_name {
                    UserConnectionAccountDao::update(sql, &UserConnectionAccount {
                        user_id: uid,
                        provider_type: provider_type.to_string(),
                        provider_account_id: connection.provider_account_id.clone(),
                        provider_account_name: bili_profile.name,
                        public: connection.public,
                        create_time: connection.create_time,
                        update_time: Utc::now(),
                    }).await?;
                }
            }
        }
        _ => bail!("Unsupported provider type"),
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use crate::service::connection_account::{random_challenge_string, CHALLENGE_CHARS};

    #[test]
    fn test_random_challenge_string() {
        let challenge = random_challenge_string();
        println!("{}", challenge);
        assert_eq!(challenge.chars().count(), 8);
        assert!(challenge.chars().all(|c| CHALLENGE_CHARS.contains(&c)));
    }
}