pub mod common;

use crate::common::{assert_is_err, assert_is_ok, CommonParse};
use common::with_test_environment;
use hachimi_world_server::web::result::WebResponse;
use hachimi_world_server::web::routes::auth::{DeviceListResp, DeviceLogoutReq, EmailRegisterReq, LoginReq, LoginResp, RefreshTokenReq, ResetPasswordReq, TokenPair};
use reqwest::StatusCode;
use serde_json::json;
use hachimi_world_server::service;
use crate::common::auth::{generate_pass_captcha_key, generate_pass_verification_code};

#[tokio::test]
async fn test_send_verification_code() {
    with_test_environment(|env| async move {
        let resp = env
            .api
            .post(
                "/auth/send_email_code",
                &json!({
                    "email": "test@example.com"
                }),
            )
            .await;
        assert_is_ok(resp).await;
        ()
    })
        .await;
}

#[tokio::test]
async fn test_register_and_login() {
    with_test_environment(|mut env| async move {
        let random_email = format!("test_{}@mail.com", uuid::Uuid::new_v4());

        // TODO[test]: Mock email sender?
        // TODO[test]: Separate these tests

        // Put a fake email code for test
        let code = generate_pass_verification_code(&mut env.redis, &random_email).await;

        let captcha_key = generate_pass_captcha_key(&env.api).await;
        // Test register with code
        let resp = env.api.post(
            "/auth/register/email",
            &EmailRegisterReq {
                email: random_email.clone(),
                password: "test12345678".to_string(),
                code,
                device_info: "test".to_string(),
                captcha_key,
            },
        ).await;
        assert_is_ok(resp).await;

        // Test login with an error password
        let captcha_key = generate_pass_captcha_key(&env.api).await;
        let resp = env.api.post(
            "/auth/login/email",
            &LoginReq {
                email: random_email.clone(),
                password: "1234".to_string(),
                device_info: "test".to_string(),
                code: None,
                captcha_key,
            },
        ).await;
        assert_is_err(resp).await;

        // Test login with the correct password
        let captcha_key = generate_pass_captcha_key(&env.api).await;
        let resp: LoginResp = env.api.post(
            "/auth/login/email",
            &LoginReq {
                email: random_email.clone(),
                password: "test12345678".to_string(),
                device_info: "test".to_string(),
                code: None,
                captcha_key,
            },
        ).await.parse_resp::<LoginResp>().await.unwrap();

        println!("{:?}", resp);
        let token = resp.token;

        // Test refresh token
        let new_token: TokenPair = env.api.post("/auth/refresh_token", &RefreshTokenReq {
            refresh_token: token.refresh_token,
            device_info: "test".to_string(),
        }).await.parse_resp::<TokenPair>().await.unwrap();

        // Test get logged device list
        env.api.set_token(new_token.access_token.clone());
        let resp: DeviceListResp = env.api.get("/auth/device/list").await.parse_resp().await.unwrap();
        assert_eq!(2, resp.devices.len());
        let last_device = resp.devices.last().unwrap();

        // Test revoke device
        let resp = env.api.post("/auth/device/logout", &DeviceLogoutReq {
            device_id: last_device.id
        }).await;
        assert_is_ok(resp).await;

        // Test refresh token with revoked, expected error
        let resp = env.api.post("/auth/refresh_token",  &RefreshTokenReq {
            refresh_token: new_token.refresh_token,
            device_info: "test".to_string(),
        }).await;
        assert_is_err(resp).await;

        // Test reset password
        let captcha_key = generate_pass_captcha_key(&env.api).await;
        service::verification_code::set_code(&mut env.redis, &random_email, "12345678").await.unwrap();
        let resp = env.api.post("/auth/reset_password", &ResetPasswordReq {
            email: random_email.to_string(),
            code: "12345678".to_string(),
            new_password: "test-changed".to_string(),
            logout_all_devices: true,
            captcha_key,
        }).await;
        assert_is_ok(resp).await;

        // Test login with the new password
        let captcha_key = generate_pass_captcha_key(&env.api).await;
        let resp = env.api.post("/auth/login/email", &LoginReq {
            email: random_email.to_string(),
            password: "test-changed".to_string(),
            device_info: "test".to_string(),
            code: None,
            captcha_key,
        }).await;
        assert_is_ok(resp).await;
    }).await;
}

#[tokio::test]
async fn test_access_protected_url_without_token() {
    with_test_environment(|env| async move {
        let resp = env.api.get("/auth/protected").await;
        assert_eq!(StatusCode::UNAUTHORIZED, resp.status());
    })
        .await;
}

#[tokio::test]
async fn test_access_with_expired_token() {
    with_test_environment(|mut env| async move {
        env.api.set_token("".into());

        let resp = env.api.get("/auth/protected").await;
        assert_eq!(StatusCode::UNAUTHORIZED, resp.status());
    })
        .await;
}

#[tokio::test]
async fn test_refresh_token() {
    // TODO: How to mock refresh tokens?
}