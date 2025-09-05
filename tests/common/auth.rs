use redis::aio::ConnectionManager;
use crate::common::{assert_is_ok, ApiClient, CommonParse, TestEnvironment};
use hachimi_world_server::service;
use hachimi_world_server::web::routes::auth::{EmailRegisterReq, EmailRegisterResp, GenerateCaptchaResp, SubmitCaptchaReq, TokenPair};

pub struct TestUser {
    pub uid: i64,
    pub email: String,
    pub token: TokenPair,
}

pub async fn with_new_random_test_user(env: &mut TestEnvironment) -> TestUser {
    let random_email = format!("test_{}@mail.com", uuid::Uuid::new_v4());
    with_new_test_user(env, &random_email).await
}

pub async fn with_new_test_user(env: &mut TestEnvironment, email: &str) -> TestUser {
    // Put a fake email code for test
    service::verification_code::set_code(&mut env.redis, &email, "12345678")
        .await
        .unwrap();
    
    // Test registering with code
    let captcha_key = generate_pass_captcha_key(&env.api).await;
    let reg_resp = env.api.post(
        "/auth/register/email",
        &EmailRegisterReq {
            email: email.to_string(),
            password: "test12345678".to_string(),
            code: "12345678".to_string(),
            device_info: "test".to_string(),
            captcha_key,
        },
    ).await.parse_resp::<EmailRegisterResp>().await.unwrap();

    env.api.set_token(reg_resp.token.access_token.clone());

    TestUser {
        uid: reg_resp.uid,
        email: email.to_string(),
        token: reg_resp.token,
    }
}

/// Make sure using test-captcha environment in integrated tests
pub async fn generate_pass_captcha_key(api: &ApiClient) -> String {
    let captcha_key = api.get("/auth/captcha/generate").await.parse_resp::<GenerateCaptchaResp>().await.unwrap();

    // Submit the test token, see cloudflare turnstile doc [Testing](https://developers.cloudflare.com/turnstile/troubleshooting/testing/)
    let r = api.post("/auth/captcha/submit", &SubmitCaptchaReq {
        captcha_key: captcha_key.captcha_key.clone(),
        token: "XXXX.DUMMY.TOKEN.XXXX".to_string()
    }).await;
    assert_is_ok(r).await;
    captcha_key.captcha_key
}

/// Generate a fake verification code for testing, directly set to "12345678" in redis
pub async fn generate_pass_verification_code(redis: &mut ConnectionManager, email: &str) -> String {
    service::verification_code::set_code(redis, email, "12345678").await.unwrap();
    "12345678".to_string()
}