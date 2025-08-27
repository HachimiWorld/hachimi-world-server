use redis::AsyncCommands;
use serde_json::json;
use crate::web::routes::auth::TurnstileCfg;

const STATUS_INIT: &str = "0";
const STATUS_SUCCESS: &str = "1";
const STATUS_FAILURE: &str = "2";

pub async fn generate_new_captcha(redis: &mut redis::aio::ConnectionManager) -> anyhow::Result<String> {
    let key = uuid::Uuid::new_v4().to_string();
    let _: () = redis.set_ex(build_captcha_redis_key(&key), STATUS_INIT, 300).await?;
    Ok(key)
}

pub async fn submit_captcha(
    cfg: &TurnstileCfg,
    redis: &mut redis::aio::ConnectionManager,
    captcha_key: &str,
    token: &str,
) -> anyhow::Result<bool> {
    let redis_key = build_captcha_redis_key(captcha_key);
    let status: Option<String> = redis.get(&redis_key).await?;
    match status {
        Some(status) => {
            if status == STATUS_INIT {
                // Verify
                let client = reqwest::Client::new();
                let verify_resp = client.post("https://challenges.cloudflare.com/turnstile/v0/siteverify")
                    .json(&json!({
                        "secret": cfg.secret_key,
                        "response": token,
                    }))
                    .send().await?;
                if verify_resp.status().is_success() {
                    let _: () = redis.set_ex(redis_key, STATUS_SUCCESS, 300).await?;
                    Ok(true)
                } else {
                    let _: () = redis.set_ex(redis_key, STATUS_FAILURE, 300).await?;
                    Ok(false)
                }
            } else {
                Ok(false)
            }
        }
        None => {
            Ok(false)
        }
    }
}

pub async fn verify_captcha(
    redis: &mut redis::aio::ConnectionManager,
    captcha_key: &str,
) -> anyhow::Result<bool> {
    let captcha_status: Option<String> = redis.get(build_captcha_redis_key(captcha_key)).await?;

    if let Some(x) = captcha_status && x == STATUS_SUCCESS {
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn build_captcha_redis_key(captcha_key: &str) -> String {
    format!("auth:captcha:{}", captcha_key)
}