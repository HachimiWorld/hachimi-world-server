use axum::http::HeaderMap;
use redis::aio::ConnectionManager;
use reqwest::Response;
use serde_json::Value;
use sqlx::PgPool;
use std::env;

pub struct TestEnvironment {
    pub api: ApiClient,
    pub pool: PgPool,
    pub redis: ConnectionManager
}

pub async fn with_test_environment<F, Fut>(f: F)
where
    F: Fn(TestEnvironment) -> Fut,
    Fut: Future<Output = ()> + Send + 'static
{
    // TODO: Launch a test server?
    dotenv::dotenv().ok();
    let api = ApiClient::new(env::var("TEST_HTTP_BASE_URL").unwrap());
    let pool = get_sql_pool().await;
    let redis = get_redis_conn().await;
    f(TestEnvironment { api, pool, redis }).await
}

pub async fn get_redis_conn() -> ConnectionManager {
    dotenv::dotenv().ok();
    let url = env::var("TEST_REDIS_URL").unwrap();
    let redis = redis::Client::open(url).unwrap();
    redis.get_connection_manager().await.unwrap()
}

pub async fn get_sql_pool() -> PgPool {
    dotenv::dotenv().ok();
    let url = env::var("DATABASE_URL").unwrap();

    let pool = PgPool::connect(&url).await.unwrap();
    pool
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
        let mut headers = HeaderMap::new();
        headers.insert("X-Real-IP", "127.0.0.1".parse().unwrap());
        if let Some(token) = &self.token {
            headers.insert(
                "Authorization",
                format!("Bearer {}", token).parse().unwrap(),
            );
        }

        let resp = client
            .get(format!("{}{path}", self.base_url))
            .headers(headers)
            .send()
            .await
            .unwrap();
        resp
    }

    pub async fn post(&self, path: &str, body: &Value) -> Response {
        let client = reqwest::Client::new();

        let mut headers = HeaderMap::new();
        headers.insert("X-Real-IP", "127.0.0.1".parse().unwrap());
        if let Some(token) = &self.token {
            headers.insert(
                "Authorization",
                format!("Bearer {}", token).parse().unwrap(),
            );
        }

        let resp = client
            .post(format!("{}{path}", self.base_url))
            .headers(headers)
            .json(body)
            .send()
            .await
            .unwrap();
        println!("{}", resp.status());
        resp
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
