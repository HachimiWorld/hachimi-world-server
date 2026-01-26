use crate::common;
use crate::config::Config;
use crate::db::user::{IUserDao, UserDao};
use crate::util::redlock::RedLock;
use crate::web::result::{CommonError, WebError};
use crate::web::state::AppState;
use anyhow::bail;
use metrics::counter;
use redis::aio::ConnectionManager;
use redis::AsyncTypedCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashSet;
use std::time::Duration;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityCfg {
    pub contributors: Vec<String>,
}

pub async fn ensure_contributor(
    state: &AppState,
    uid: i64,
) -> Result<(), WebError<CommonError>> {
    let config = state.config.clone();
    let pool = &state.sql_pool;
    let redis = state.redis_conn.clone();
    let is_contributor = check_contributor(&config, redis, &state.red_lock, pool, uid).await?;

    if is_contributor {
        Ok(())
    } else {
        Err(common!("permission_denied", "You are not a contributor"))
    }
}

pub async fn check_contributor(
    config: &Config,
    mut redis: ConnectionManager,
    red_lock: &RedLock,
    pool: &PgPool,
    uid: i64,
) -> anyhow::Result<bool> {
    let contributors = redis.get("contributors").await?;
    if let Some(contributors) = contributors {
        counter!("check_contributor_cache_hit_count").increment(1);
        let contributor_uids: Vec<i64> = serde_json::from_str(&contributors)?;
        Ok(contributor_uids.contains(&uid))
    } else {
        counter!("check_contributor_cache_miss_count").increment(1);

        let lock = red_lock.lock_with_timeout("lock:contributors", Duration::from_secs(30)).await?;
        if lock.is_none() {
            counter!("check_contributor_lock_timeout_count").increment(1);
            bail!("Can't get lock")
        }

        // Check cache again
        let contributors = redis.get("contributors").await?;
        if let Some(contributors) = contributors {
            let contributor_uids: Vec<i64> = serde_json::from_str(&contributors)?;
            if contributor_uids.contains(&uid) {
                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            // Get from source of truth
            // TODO: Get from github repository
            let cfg: CommunityCfg = config.get_and_parse("community")?;
            let mut contributor_uids = HashSet::new();
            for email in cfg.contributors {
                if let Some(user) = UserDao::get_by_email(pool, &email).await? {
                    contributor_uids.insert(user.id);
                } else {
                    warn!("Contributor {} was configured but not found in database", email);
                }
            }
            redis.set("contributors", serde_json::to_string(&contributor_uids)?).await?;
            if contributor_uids.contains(&uid) {
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }
}