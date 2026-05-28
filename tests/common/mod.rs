pub mod auth;
pub mod song;

use axum::http::HeaderMap;
use hachimi_world_server::config::Config;
use hachimi_world_server::file_hosting::{FileHost, MockFileHost, UploadResult};
use hachimi_world_server::util::redlock::RedLock;
use hachimi_world_server::web::result::CommonError;
use hachimi_world_server::web::state::AppState;
use hachimi_world_server::web::{run_web_app, ServerCfg};
use redis::aio::ConnectionManager;
use reqwest::{RequestBuilder, Response};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use std::env;
use std::sync::Arc;
use testcontainers_modules::redis::REDIS_PORT;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tracing::info;

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
    dotenv::dotenv().unwrap();

    let server_cfg = ServerCfg {
        listen: "localhost:20080".to_string(),
        metrics_listen: "localhost:0".to_string(),
        jwt_secret: "12345678".to_string(),
        allow_origins: vec!["http://localhost".to_string()],
        publish_version_token: "12345678".to_string(),
    };

    let app_state = get_test_app_state().await;

    let _handle = tokio::spawn(run_web_app(server_cfg, app_state.clone(), tokio_util::sync::CancellationToken::new()));
    let api = ApiClient::new("http://localhost:20080".to_string());

    f(TestEnvironment { api, pool: app_state.sql_pool.clone(), redis: app_state.redis_conn.clone() }).await
}

async fn get_test_redis_conn() -> ConnectionManager {
    let redis_instance = testcontainers_modules::redis::Redis::default().start().await.unwrap();
    let host_ip = redis_instance.get_host().await.unwrap();
    let host_port = redis_instance.get_host_port_ipv4(REDIS_PORT).await.unwrap();
    let redis_url = format!("redis://{host_ip}:{host_port}");
    info!("Open test redis instance at {}", redis_url);
    let redis = redis::Client::open(redis_url).unwrap();
    redis.get_connection_manager().await.unwrap()
}

async fn get_test_sql_pool() -> PgPool {
    let instance = testcontainers_modules::postgres::Postgres::default().start().await.unwrap();
    let host_ip = instance.get_host().await.unwrap();
    let host_port = instance.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host_ip}:{host_port}/postgres");
    info!("Open test postgres instance at {}", url);
    PgPool::connect(&url).await.unwrap()
}

async fn get_test_meilisearch() -> meilisearch_sdk::client::Client {
    let instance = testcontainers_modules::meilisearch::Meilisearch::default().start().await.unwrap();
    let host_ip = instance.get_host().await.unwrap();
    let host_port = instance.get_host_port_ipv4(7700).await.unwrap();
    let url = format!("http://{host_ip}:{host_port}");
    info!("Open test meilisearch instance at {}", url);
    meilisearch_sdk::client::Client::new(url, Some("")).unwrap()
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
    }).returning(|bytes, key| {
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
    Config::parse_by_str("").unwrap()
}

async fn get_test_app_state() -> AppState {
    let redis_conn = get_test_redis_conn().await;
    AppState {
        sql_pool: get_test_sql_pool().await,
        file_host: Arc::new(get_test_file_host().await),
        meilisearch: Arc::new(get_test_meilisearch().await),
        redis_conn: redis_conn.clone(),
        config: Arc::new(get_test_config()),
        red_lock: RedLock::new(redis_conn).unwrap(),
    }
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

    pub async fn get(&self, path: &str) -> Response {
        let client = reqwest::Client::new();

        let resp = client
            .get(format!("{}{path}", self.base_url))
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
            .get(format!("{}{path}", self.base_url))
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
            .post(format!("{}{path}", self.base_url))
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
        client.post(format!("{}{path}", self.base_url))
            .headers(self.default_headers())
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

pub trait CommonParse {
    async fn parse_resp<T: for<'de> serde::Deserialize<'de>>(self) -> ApiResult<T>;
}

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