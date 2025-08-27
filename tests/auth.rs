pub mod common;

use crate::common::{assert_is_err, assert_is_ok};
use common::with_test_environment;
use hachimi_world_server::web::result::WebResponse;
use hachimi_world_server::web::routes::auth::{DeviceListResp, LoginReq, LoginResp, ResetPasswordReq, TokenPair};
use reqwest::StatusCode;
use serde_json::json;
use hachimi_world_server::service;

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
        service::verification_code::set_code(&mut env.redis, &random_email, "12345678").await.unwrap();

        // Test register with code
        let resp = env
            .api
            .post(
                "/auth/register/email",
                &json!({
                    "email": random_email,
                    "password": "test12345678",
                    "code": "12345678",
                    "device_info": "test",
                }),
            )
            .await;
        assert_is_ok(resp).await;

        // Test login with error password
        let resp = env.api.post(
            "/auth/login/email",
            &json!({
                "email": random_email,
                "password": "1234",
                "device_info": "test"
            }),
        ).await;
        assert_is_err(resp).await;

        let resp: WebResponse<LoginResp> = env.api.post(
            "/auth/login/email",
            &json!({
                "email": random_email,
                "password": "test12345678",
                "device_info": "test"
            }),
        ).await.json().await.unwrap();
        println!("{:?}", resp);
        let token = resp.data.token;

        // Test refresh token
        let new_token: WebResponse<TokenPair> = env.api.post("/auth/refresh_token", &json!({
            "refresh_token": token.refresh_token,
            "device_info": "test"
        })).await.json().await.unwrap();

        // Test get logged device list
        env.api.set_token(new_token.data.access_token);
        let resp: WebResponse<DeviceListResp> = env.api.get("/auth/device/list").await.json().await.unwrap();
        assert_eq!(2, resp.data.devices.len());
        let last_device = resp.data.devices.last().unwrap();

        // Test revoke device
        let resp = env.api.post("/auth/device/logout", &json!({
            "device_id": last_device.id
        })).await;
        assert_is_ok(resp).await;

        // Test refresh token with revoked, expected error
        let resp = env.api.post("/auth/refresh_token", &json!({
            "refresh_token": new_token.data.refresh_token,
            "device_info": "test"
        })).await;
        assert_is_err(resp).await;

        // Test reset password
        service::verification_code::set_code(&mut env.redis, &random_email, "12345678").await.unwrap();
        let resp = env.api.post("/auth/reset_password", &ResetPasswordReq {
            email: random_email.to_string(),
            code: "12345678".to_string(),
            new_password: "test-changed".to_string(),
            logout_all_devices: true,
        }).await;
        assert_is_ok(resp).await;

        // Test login with new password
        let resp = env.api.post("/auth/login/email", &LoginReq {
            email: random_email.to_string(),
            password: "test-changed".to_string(),
            device_info: "test".to_string(),
            code: None,
            captcha_key: "test_captcha_key".to_string(), // TODO[test]: Add a way to mock captcha key
        }).await;
        assert_is_ok(resp).await;
    })
    .await;
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