extern crate hachimi_world_server as app;

use std::sync::Arc;
use async_backtrace::framed;
use serde::{Deserialize, Serialize};
use app::util::gracefully_shutdown;
#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;
use tracing::{info, info_span, Instrument};
use app::config::Config;
use app::file_hosting::FileHost;
use app::{search, web};
use app::web::state::AppState;
use aws_sdk_s3 as s3;
use aws_sdk_s3::config::Region;
use app::web::ServerCfg;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[framed]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let (cancel_token, cancel_handle) = gracefully_shutdown::gen_cancel_token();
    let config = Config::parse("config.yaml")?;

    let server_cfg = config.get_and_parse::<ServerCfg>("server")?;

    let all = async {
        tokio::join!(
            get_redis_pool(config.clone()),
            get_database_pool(config.clone()),
            get_file_host(config.clone()),
            get_meilisearch_client(config.clone())
        )
    };

    let state = tokio::select! {
        (redis_conn, sql_pool, file_host, meilisearch_client) = all => {
            AppState {
                redis_conn: redis_conn?,
                config: Arc::new(config),
                sql_pool: sql_pool?,
                file_host: Arc::new(file_host?),
                meilisearch: Arc::new(meilisearch_client?)
            }
        }
        _ = cancel_token.cancelled() => {
            info!("Shutdown");
            return Ok(())
        }
    };

    // Initialize auth service

    info!("Starting web server at {}", server_cfg.listen);
    web::run_web_app(server_cfg, state, cancel_token).await?;

    cancel_handle.await?;
    info!("Shutdown successfully");
    Ok(())
}

#[derive(Deserialize, Clone, Debug)]
struct DatabaseConfig {
    pub address: String,
    pub username: String,
    pub password: String,
    pub database: String,
}


async fn get_database_pool(config: Config) -> anyhow::Result<sqlx::PgPool> {
    let span = info_span!("database");
    async {
        // <type>://<username>:<password>@<host>[:<port>][/[<db>][?<params>]]
        let DatabaseConfig {
            address,
            username,
            password,
            database,
        } = config.get_and_parse::<DatabaseConfig>("db")?;

        let url = format!(
            "postgres://{username}:{password}@{address}/{database}",
            password = urlencoding::encode(&password),
        );
        info!("Connecting to postgresql at {address}");
        let sql_pool = sqlx::PgPool::connect(&url).await?;

        // Run migrations
        // TODO: Consider to integrate with CI?
        info!("Running migrations");
        sqlx::migrate!().run(&sql_pool).await?;

        info!("Database connected");
        Ok(sql_pool)
    }.instrument(span).await
}

#[derive(Deserialize, Clone, Debug)]
struct RedisConfig {
    pub address: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub database: Option<u16>,
}

async fn get_redis_pool(config: Config) -> anyhow::Result<redis::aio::ConnectionManager> {
    let span = info_span!("redis");
    async {
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
    }.instrument(span).await
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct S3Config {
    pub bucket_name: String,
    pub endpoint_url: String,
    pub public_domain: String,
    pub access_key_id: String,
    pub access_key_secret: String,
}

async fn get_file_host(config: Config) -> anyhow::Result<FileHost> {
    let cfg: S3Config = config.get_and_parse("s3")?;

    // Configure the client
    let config = s3::Config::builder()
        .endpoint_url(cfg.endpoint_url)
        .credentials_provider(aws_sdk_s3::config::Credentials::new(
            cfg.access_key_id,
            cfg.access_key_secret,
            None, // session token is not used with R2
            None,
            "R2",
        ))
        .region(Region::new("auto"))
        .behavior_version_latest()
        .build();

    let client = s3::Client::from_conf(config);
    Ok(FileHost::new(
        cfg.bucket_name,
        cfg.public_domain,
        client,
    ))
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct MeiliCfg {
    pub host: String,
    pub api_key: String,
}

async fn get_meilisearch_client(config: Config) -> anyhow::Result<meilisearch_sdk::client::Client> {
    let cfg: MeiliCfg = config.get_and_parse("meilisearch")?;
    let client = meilisearch_sdk::client::Client::new(cfg.host, Some(cfg.api_key))?;
    let span = info_span!("search");
    async {
        info!("Setting up search index");
        search::setup_search_index(&client).await
    }.instrument(span).await?;
    Ok(client)
}