use crate::config::Config;
use redis::aio::ConnectionManager;
use sqlx::{Pool, Postgres};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub redis_conn: ConnectionManager,
    pub config: Arc<Config>,
    pub sql_pool: Pool<Postgres>,
}
