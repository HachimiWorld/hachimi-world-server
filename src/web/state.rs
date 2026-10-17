use crate::config::Config;
use crate::file_hosting::FileHost;
use crate::service::content_check::ContentChecker;
use crate::util::bilibili::BilibiliClient;
use crate::util::redlock::RedLock;
use redis::aio::ConnectionManager;
use sqlx::{Pool, Postgres};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub redis_conn: ConnectionManager,
    pub config: Arc<Config>,
    pub sql_pool: Pool<Postgres>,
    pub file_host: Arc<dyn FileHost>,
    pub meilisearch: Arc<meilisearch_sdk::client::Client>,
    pub red_lock: RedLock,
    pub bili_client: Arc<dyn BilibiliClient>,
    /// `None` when `content_check` isn't configured: content isn't checked.
    /// @since 261016
    pub content_checker: Option<Arc<ContentChecker>>,
}
