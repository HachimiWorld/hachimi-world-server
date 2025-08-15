extern crate hachimi_world_server as app;

use std::sync::Arc;
use serde::Deserialize;
use app::util::gracefully_shutdown;
#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;
use tracing::info;
use app::config::Config;
use app::web;
use app::web::state::AppState;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let (cancel_token, cancel_handle) = gracefully_shutdown::gen_cancel_token();
    let config = Config::parse("config.yaml")?;

    let server_cfg = config.get_and_parse::<ServerCfg>("server")?;
    let redis_conn = get_redis_pool(&config).await?;
    let sql_pool = get_database_pool(&config).await?;
    let state = AppState {
        redis_conn,
        config: Arc::new(config),
        sql_pool,
    };

    info!("Starting web server at {}", server_cfg.listen);
    web::run_web_app(
        state,
        server_cfg.listen,
        server_cfg.metrics_listen,
        cancel_token,
    ).await?;

    cancel_handle.await?;
    info!("Shutdown successfully");
    Ok(())
}

#[derive(Deserialize)]
struct ServerCfg {
    listen: String,
    metrics_listen: String,
}

#[derive(Deserialize, Clone, Debug)]
struct DatabaseConfig {
    pub address: String,
    pub username: String,
    pub password: String,
    pub database: String,
}


async fn get_database_pool(config: &Config) -> anyhow::Result<sqlx::PgPool> {
    // <type>://<username>:<password>@<host>[:<port>][/[<database>][?<params>]]
    let DatabaseConfig {
        address,
        username,
        password,
        database,
    } = config.get_and_parse::<DatabaseConfig>("database")?;

    let url = format!(
        "mysql://{username}:{password}@{address}/{database}",
        password = urlencoding::encode(&password),
    );
    info!("Connecting to mysql at {address}");
    let sql_pool = sqlx::PgPool::connect(&url).await?;

    // Run migrations
    // TODO: Consider to integrate with CI?
    info!("Running migrations");
    sqlx::migrate!().run(&sql_pool).await?;

    info!("Database connected");
    Ok(sql_pool)
}

#[derive(Deserialize, Clone, Debug)]
struct RedisConfig {
    pub address: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub database: Option<u16>,
}

async fn get_redis_pool(config: &Config) -> anyhow::Result<redis::aio::ConnectionManager> {
    // redis://[<username>][:<password>@]<hostname>[:<port>][/[<db>][?protocol=<protocol>]]
    let config = config.get_and_parse::<RedisConfig>("redis")?;

    let url = format!(
        "redis://{username}{password}{address}{database}",
        username = config.username.map_or(String::new(), |u| u),
        password = config.password.map_or(String::new(), |p| format!(
            ":{p}@",
            p = urlencoding::encode(&p)
        )),
        address = config.address,
        database = config.database.map_or(String::new(), |d| format!("/{d}"))
    );
    info!("Connecting to redis at {}", config.address);
    let redis = redis::Client::open(url)?;
    let redis_conn = redis.get_connection_manager().await?;
    info!("Redis connected");
    Ok(redis_conn)
}
