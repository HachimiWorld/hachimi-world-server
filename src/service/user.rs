use crate::db::user::{IUserDao, UserDao};
use crate::web::routes::user::PublicUserProfile;
use itertools::Itertools;
use redis::aio::ConnectionManager;
use sqlx::PgPool;
use std::collections::HashMap;

pub async fn get_public_profile(
    redis: ConnectionManager,
    sql_pool: &PgPool,
    user_ids: &[i64]
) -> sqlx::Result<HashMap<i64, PublicUserProfile>> {
    // TODO: Cache user
    let unique_uids = user_ids.iter().copied().unique().collect_vec();
    let users = UserDao::list_by_ids(sql_pool, &unique_uids).await?;

    let profiles: HashMap<_, _> = users.into_iter()
        .map(|u| PublicUserProfile {
            uid: u.id,
            username: u.username,
            avatar_url: u.avatar_url,
            bio: u.bio,
            gender: u.gender,
            is_banned: u.is_banned,
        })
        .into_iter()
        .map(|x| (x.uid, x))
        .collect();
    Ok(profiles)
}