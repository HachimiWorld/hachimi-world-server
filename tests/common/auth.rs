use crate::common::TestEnvironment;
use hachimi_world_server::service;
use hachimi_world_server::web::result::WebResponse;
use hachimi_world_server::web::routes::auth::{
    EmailRegisterReq, EmailRegisterResp, TokenPair,
};

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

    // Test register with code
    let reg_resp = env
        .api
        .post(
            "/auth/register/email",
            &EmailRegisterReq {
                email: email.to_string(),
                password: "test12345678".to_string(),
                code: "12345678".to_string(),
                device_info: "test".to_string(),
                captcha_key: "".to_string(), // TODO[test]: Add a way to mock captcha key
            },
        )
        .await
        .json::<WebResponse<EmailRegisterResp>>()
        .await
        .unwrap();

    env.api.set_token(reg_resp.data.token.access_token.clone());

    TestUser {
        uid: reg_resp.data.uid,
        email: email.to_string(),
        token: reg_resp.data.token,
    }
}
