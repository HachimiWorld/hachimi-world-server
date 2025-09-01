use crate::config::Config;
use redis::aio::ConnectionManager;
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use crate::file_hosting::FileHost;
use crate::util::redlock::RedLock;

#[derive(Clone)]
pub struct AppState {
    pub redis_conn: ConnectionManager,
    pub config: Arc<Config>,
    pub sql_pool: Pool<Postgres>,
    pub file_host: Arc<FileHost>,
    pub meilisearch: Arc<meilisearch_sdk::client::Client>,
    pub red_lock: RedLock
}
