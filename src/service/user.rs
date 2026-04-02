use crate::db::user::{IUserDao, UserDao};
use crate::service::connection_account;
use crate::service::connection_account::ConnectionAccount;
use crate::web::routes::user::{ConnectedAccountItem, PublicUserProfile};
use itertools::Itertools;
use redis::aio::ConnectionManager;
use redis::{AsyncTypedCommands, MSetOptions, SetExpiry};
use sqlx::PgPool;
use std::collections::HashMap;

pub async fn get_public_profile(
    redis: ConnectionManager,
    sql_pool: &PgPool,
    user_ids: &[i64],
) -> anyhow::Result<HashMap<i64, PublicUserProfile>> {
    let unique_uids = user_ids.iter().copied().unique().collect_vec();
    let mut cached_profiles = get_from_cache(redis.clone(), &unique_uids).await?;

    let missed_ids = unique_uids.into_iter().filter(|uid| !cached_profiles.contains_key(uid)).collect_vec();
    if missed_ids.is_empty() {
        return Ok(cached_profiles);
    }

    let users = UserDao::list_by_ids(sql_pool, &missed_ids).await?;

    // parallel get connections for each user and fill in the profile, but for now just return empty connections
    let connections: HashMap<i64, Vec<ConnectionAccount>> = futures::future::join_all(users.iter().map(|u| {
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
        })
        .into_iter()
        .map(|x| (x.uid, x))
        .collect();
    save_to_cache(redis, &profiles).await?;
    cached_profiles.extend(profiles);
    Ok(cached_profiles)
}

fn gen_cache_key(uid: i64) -> String {
    format!("user_profile:uid={}", uid)
}

async fn get_from_cache(mut redis: ConnectionManager, user_ids: &[i64]) -> anyhow::Result<HashMap<i64, PublicUserProfile>> {
    // mget
    let keys: Vec<String> = user_ids.iter().map(|uid| gen_cache_key(*uid)).collect();
    let values: Vec<Option<String>> = redis.mget(&keys).await?;
    let result: HashMap<i64, PublicUserProfile> = user_ids.iter().cloned().zip(values.into_iter())
        .filter_map(|(uid, value)| {
            value.and_then(|v| serde_json::from_str::<PublicUserProfile>(&v).ok()).map(|profile| (uid, profile))
        })
        .collect();
    Ok(result)
}

async fn save_to_cache(mut redis: ConnectionManager, profiles: &HashMap<i64, PublicUserProfile>) -> anyhow::Result<()> {
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