pub mod auth;
pub mod song;

use axum::http::HeaderMap;
use hachimi_world_server::web::result::CommonError;
use redis::aio::ConnectionManager;
use reqwest::{RequestBuilder, Response};
use serde::Serialize;
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

        let query = serde_json::to_value(query).unwrap();
        let resp = client
            .get(format!("{}{path}", self.base_url))
            .headers(self.default_headers())
            .query(&query)
            .send()
            .await
            .unwrap();
        println!("[{}] GET to {}; Query: {}", resp.status(), path, query.to_string());
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