#![allow(dead_code)]

pub mod auth;
pub mod song;
pub mod bilibili;
pub mod res_utils;
pub mod publish;
pub mod images;

use crate::common::images::{Meilisearch1_32, Postgres17, Redis8};
use async_trait::async_trait;
use axum::http::HeaderMap;
use hachimi_world_server::config::Config;
use hachimi_world_server::file_hosting::{FileHost, MockFileHost, UploadResult};
use hachimi_world_server::search::setup_meilisearch_indexes;
use hachimi_world_server::util::redlock::RedLock;
use hachimi_world_server::web::result::CommonError;
use hachimi_world_server::web::state::AppState;
use hachimi_world_server::web::{start_main_server, ServerCfg};
use redis::aio::ConnectionManager;
use reqwest::{RequestBuilder, Response};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use testcontainers_modules::redis::REDIS_PORT;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ContainerAsync;
use tokio::net::TcpListener;
use tracing::{info, Level};

pub struct TestEnvironment {
    pub api: ApiClient,
    pub pool: PgPool,
    pub redis: ConnectionManager,
}


/// This is used to start a black-box test environment. It will launch the web server and provide an API client, SQL pool and Redis connection for testing.
pub async fn with_test_environment<F, Fut>(f: F)
where
    F: Fn(TestEnvironment) -> Fut,
    Fut: Future<Output=()> + Send + 'static,
{
    tracing_subscriber::fmt().with_max_level(Level::INFO).try_init().ok(); // Ignore error
    dotenv::dotenv().ok();

    let server_cfg = ServerCfg {
        listen: "localhost:0".to_string(),
        metrics_listen: "localhost:0".to_string(),
        jwt_secret: "12345678".to_string(),
        allow_origins: vec!["http://localhost".to_string()],
        publish_version_token: "12345678".to_string(),
    };

    // Boot external dependencies in parallel to reduce test startup time.
    let ((redis_instance, redis_conn), (postgres_instance, sql_pool), (ms_instance, ms_client)) = tokio::join!(
        get_test_redis_conn(),
        async {
            let (postgres_instance, sql_pool) = get_test_sql_pool().await;
            // Migrations depend on Postgres only, so run them in the same task.
            sqlx::migrate!().run(&sql_pool).await.unwrap();
            (postgres_instance, sql_pool)
        },
        get_test_meilisearch(),
    );

    setup_meilisearch_indexes(&ms_client, &sql_pool).await.unwrap();

    let app_state = AppState {
        sql_pool: sql_pool,
        file_host: Arc::new(get_test_file_host().await),
        meilisearch: Arc::new(ms_client),
        redis_conn: redis_conn.clone(),
        config: Arc::new(get_test_config()),
        red_lock: RedLock::new(redis_conn).unwrap(),
        bili_client: Arc::new(bilibili::get_mock_bili_client()),
    };

    let random_listener = TcpListener::bind("localhost:0").await.unwrap(); // Use OS assigned port to avoid conflicts
    let random_port = random_listener.local_addr().unwrap().port();
    let server = start_main_server(
        random_listener,
        app_state.clone(),
        server_cfg.allow_origins,
        hachimi_world_server::web::jwt::Keys::new(server_cfg.jwt_secret.as_bytes()),
        server_cfg.publish_version_token,
        128,
        tokio_util::sync::CancellationToken::new(),
    );
    let _handle = tokio::spawn(server);
    let api = ApiClient::new(format!("http://localhost:{random_port}"));

    // A trick to wait for the server to start. Especially for waiting the invocation of `initialize_jwt_key`
    tokio::time::sleep(Duration::from_millis(100)).await;
    let test_env = TestEnvironment { api, pool: app_state.sql_pool.clone(), redis: app_state.redis_conn.clone() };
    f(test_env).await;

    // Hold the instances until the end of the test to make sure the connections are still valid
    drop(redis_instance);
    drop(postgres_instance);
    drop(ms_instance);
}

async fn get_test_redis_conn() -> (ContainerAsync<Redis8>, ConnectionManager) {
    let redis_instance = Redis8::default().start().await.unwrap();
    let host_ip = redis_instance.get_host().await.unwrap();
    let host_port = redis_instance.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    let redis_url = format!("redis://{host_ip}:{host_port}");
    info!("Open test redis instance at {}", redis_url);
    let redis = redis::Client::open(redis_url).unwrap();
    (redis_instance, redis.get_connection_manager().await.unwrap())
}

async fn get_test_sql_pool() -> (ContainerAsync<Postgres17>, PgPool) {
    let instance = Postgres17::default().start().await.unwrap();
    let host_ip = instance.get_host().await.unwrap();
    let host_port = instance.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host_ip}:{host_port}/postgres");
    info!("Open test postgres instance at {}", url);
    (instance, PgPool::connect(&url).await.unwrap())
}

async fn get_test_meilisearch() -> (ContainerAsync<Meilisearch1_32>, meilisearch_sdk::client::Client) {
    let instance = Meilisearch1_32::default().start().await.unwrap();
    let host_ip = instance.get_host().await.unwrap();
    let host_port = instance.get_host_port_ipv4(7700).await.unwrap();
    let url = format!("http://{host_ip}:{host_port}");
    info!("Open test meilisearch instance at {}", url);
    (instance, meilisearch_sdk::client::Client::new(url, Some("12345678")).unwrap())
}

async fn get_test_file_host() -> impl FileHost {
    // Return an mock file host since we don't want to actually upload files during tests. The file host is only used for generating file URLs, so it won't affect the tests.
    let mut mock = MockFileHost::default();
    mock.expect_rename().withf(|old_key, new_key| {
        info!("Mock rename file from {} to {}", old_key, new_key);
        true
    }).returning(|_, _| Box::pin(async move { Ok(()) }));
    mock.expect_upload().withf(|bytes, key| {
        info!("Mock upload file {} ({} bytes)", key, bytes.len());
        true
    }).returning(|_bytes, key| {
        let key = key.to_string();
        Box::pin(async move {
            Ok(UploadResult {
                output: aws_sdk_s3::operation::put_object::PutObjectOutput::builder().build(),
                public_url: format!("https://mock-file-host/{}", key),
            })
        })
    });
    mock
}


fn get_test_config() -> Config {
    Config::parse("tests/fixtures/test-config.yaml").unwrap()
}

pub struct ApiClient {
    base_url: String,
    token: Option<String>,
}

impl ApiClient {
    pub fn new(url: String) -> ApiClient {
        ApiClient {
            base_url: url,
            token: None,
        }
    }

    pub fn set_token(&mut self, token: String) {
        self.token = Some(token);
    }

    pub fn clear_token(&mut self) {
        self.token = None;
    }

    pub async fn get(&self, path: &str) -> Response {
        let client = reqwest::Client::new();

        let resp = client
            .get(self.build_url(path))
            .headers(self.default_headers())
            .send()
            .await
            .unwrap();
        println!("[{}] GET to {}", resp.status(), path);
        resp
    }

    pub async fn get_query<T: Serialize>(&self, path: &str, query: &T) -> Response {
        let client = reqwest::Client::new();

        let resp = client
            .get(self.build_url(path))
            .headers(self.default_headers())
            .query(&query)
            .send()
            .await
            .unwrap();
        println!("[{}] GET to {}; Query: {}", resp.status(), path, serde_urlencoded::to_string(query).unwrap());
        resp
    }

    pub async fn post<T: Serialize>(&self, path: &str, body: &T) -> Response {
        let client = reqwest::Client::new();
        let body = serde_json::to_value(body).unwrap();
        let resp = client
            .post(self.build_url(path))
            .headers(self.default_headers())
            .json(&body)
            .send()
            .await
            .unwrap();
        println!("[{}] POST to {}; Body: {}", resp.status(), path, body.to_string());
        resp
    }

    pub fn post_raw(&self, path: &str) -> RequestBuilder {
        let client = reqwest::Client::new();
        client.post(self.build_url(path))
            .headers(self.default_headers())
    }

    fn build_url(&self, path: &str) -> String {
        // Trick for health check endpoint, since it's not under /api prefix
        if path == "/health" {
            format!("{}{path}", self.base_url)
        } else {
            format!("{}/api{path}", self.base_url)
        }
    }

    fn default_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("X-Real-IP", "127.0.0.1".parse().unwrap());
        headers.insert("User-Agent", "test".parse().unwrap());
        if let Some(token) = &self.token {
            headers.insert(
                "Authorization",
                format!("Bearer {}", token).parse().unwrap(),
            );
        }
        headers
    }
}

pub async fn assert_is_ok(resp: Response) {
    let value: Value = resp.json().await.unwrap();
    assert_eq!(value["ok"], true, "{}", value);
}

pub async fn assert_is_err(resp: Response) {
    let value: Value = resp.json().await.unwrap();
    assert_eq!(value["ok"], false, "{}", value);
}

pub type ApiResult<T, E = CommonError> = Result<T, E>;

#[async_trait]
pub trait CommonParse {
    async fn parse_resp<T: for<'de> serde::Deserialize<'de>>(self) -> ApiResult<T>;
}

#[async_trait]
impl CommonParse for Response {
    async fn parse_resp<T: for<'de> serde::Deserialize<'de>>(self) -> ApiResult<T> {
        let text = self.text().await.unwrap();
        println!("Response: {}", text);

        let mut value: Value = serde_json::from_str(&text).unwrap();
        let data = value.get_mut("data").unwrap().take();

        if value["ok"].as_bool().unwrap() {
            let data: T = serde_json::from_value(data).unwrap();
            Ok(data)
        } else {
            let data: CommonError = serde_json::from_value(data).unwrap();
            Err(data)
        }
    }
}
