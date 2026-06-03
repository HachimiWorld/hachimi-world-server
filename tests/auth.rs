pub mod common;

use crate::common::auth::{generate_pass_captcha_key, generate_pass_verification_code, with_new_random_test_user};
use crate::common::{assert_is_err, assert_is_ok, CommonParse};
use chrono::{Duration, Utc};
use common::with_test_environment;
use hachimi_world_server::web::jwt::{generate_access_token, generate_refresh_token_with_exp};
use hachimi_world_server::web::routes::auth::{DeviceListResp, DeviceLogoutReq, EmailRegisterReq, LoginReq, LoginResp, RefreshTokenReq, ResetPasswordReq, TokenPair};
use hachimi_world_server::{service, web};
use reqwest::StatusCode;
use web::routes::auth::SendVerificationReq;

#[tokio::test]
async fn test_send_verification_code() {
    with_test_environment(|env| async move {
        let resp = env
            .api
            .post(
                "/auth/send_email_code",
                &SendVerificationReq {
                    email: "test@example.com".to_string(),
                },
            ).await;
        assert_is_ok(resp).await;
    }).await;
}

#[tokio::test]
async fn test_register_and_login() {
    with_test_environment(|mut env| async move {
        let random_email = format!("test_{}@example.com", uuid::Uuid::new_v4());

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

        // Test login with a wrong password
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
        assert!(!resp.token.access_token.is_empty());
        assert!(!resp.token.refresh_token.is_empty());
    }).await;
}

#[tokio::test]
async fn test_refresh_token() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        let new_token: TokenPair = env.api.post("/auth/refresh_token", &RefreshTokenReq {
            refresh_token: user.token.refresh_token.clone(),
            device_info: "test".to_string(),
        }).await.parse_resp::<TokenPair>().await.unwrap();

        assert!(!new_token.access_token.is_empty());
        assert!(!new_token.refresh_token.is_empty());
    }).await;
}

#[tokio::test]
async fn test_refresh_token_should_reject_expired_token() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;
        // Generate an expired refresh token for test
        let expired_refresh_token = generate_refresh_token_with_exp(
            &user.uid.to_string(),
            chrono::Utc::now() - Duration::days(1)
        );
        let resp = env.api.post("/auth/refresh_token", &RefreshTokenReq {
            refresh_token: expired_refresh_token.0,
            device_info: "test".to_string(),
        }).await.parse_resp::<TokenPair>().await.unwrap_err();
        assert_eq!("token_expired", resp.code);
    }).await;
}

#[tokio::test]
async fn test_device_management() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        let captcha_key = generate_pass_captcha_key(&env.api).await;
        let second_login: LoginResp = env.api.post(
            "/auth/login/email",
            &LoginReq {
                email: user.email.clone(),
                password: "test12345678".to_string(),
                device_info: "test-device-2".to_string(),
                code: None,
                captcha_key,
            },
        ).await.parse_resp::<LoginResp>().await.unwrap();

        env.api.set_token(second_login.token.access_token.clone());
        let resp: DeviceListResp = env.api.get("/auth/device/list").await.parse_resp().await.unwrap();
        assert_eq!(2, resp.devices.len());

        let second_device = resp
            .devices
            .iter()
            .find(|d| d.device_info.as_deref() == Some("test-device-2"))
            .expect("second device should exist");

        let resp = env.api.post("/auth/device/logout", &DeviceLogoutReq {
            device_id: second_device.id,
        }).await;
        assert_is_ok(resp).await;

        let resp = env.api.post("/auth/refresh_token", &RefreshTokenReq {
            refresh_token: second_login.token.refresh_token,
            device_info: "test-device-2".to_string(),
        }).await;
        assert_is_err(resp).await;
    }).await;
}

#[tokio::test]
async fn test_reset_password() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        let captcha_key = generate_pass_captcha_key(&env.api).await;
        service::verification_code::set_code(&mut env.redis, &user.email, "12345678").await.unwrap();
        let resp = env.api.post("/auth/reset_password", &ResetPasswordReq {
            email: user.email.clone(),
            code: "12345678".to_string(),
            new_password: "test-changed".to_string(),
            logout_all_devices: true,
            captcha_key,
        }).await;
        assert_is_ok(resp).await;

        let resp = env.api.post("/auth/refresh_token", &RefreshTokenReq {
            refresh_token: user.token.refresh_token,
            device_info: "test".to_string(),
        }).await;
        assert_is_err(resp).await;

        let captcha_key = generate_pass_captcha_key(&env.api).await;
        let resp = env.api.post("/auth/login/email", &LoginReq {
            email: user.email.clone(),
            password: "test12345678".to_string(),
            device_info: "test".to_string(),
            code: None,
            captcha_key,
        }).await;
        assert_is_err(resp).await;

        let captcha_key = generate_pass_captcha_key(&env.api).await;
        let resp = env.api.post("/auth/login/email", &LoginReq {
            email: user.email,
            password: "test-changed".to_string(),
            device_info: "test".to_string(),
            code: None,
            captcha_key,
        }).await;
        assert_is_ok(resp).await;
    }).await;
}

#[tokio::test]
async fn test_max_verification_code_retries() {
    with_test_environment(|mut env| async move {
        let random_email = format!("test_{}@example.com", uuid::Uuid::new_v4());

        // Set a known verification code
        service::verification_code::set_code(&mut env.redis, &random_email, "12345678")
            .await
            .unwrap();

        // Submit wrong code 4 times — on the 4th attempt the retry counter exceeds 3
        // and the code is automatically invalidated
        for _ in 0..4 {
            let captcha_key = generate_pass_captcha_key(&env.api).await;
            let resp = env.api.post(
                "/auth/register/email",
                &EmailRegisterReq {
                    email: random_email.clone(),
                    password: "test12345678".to_string(),
                    code: "000000".to_string(),
                    device_info: "test".to_string(),
                    captcha_key,
                },
            ).await;
            assert_is_err(resp).await;
        }

        // Now the correct code should also be rejected because it was invalidated
        let captcha_key = generate_pass_captcha_key(&env.api).await;
        let resp = env.api.post(
            "/auth/register/email",
            &EmailRegisterReq {
                email: random_email.clone(),
                password: "test12345678".to_string(),
                code: "12345678".to_string(),
                device_info: "test".to_string(),
                captcha_key,
            },
        ).await;
        assert_is_err(resp).await;
    }).await;
}


#[tokio::test]
async fn test_access_protected_url_without_token() {
    with_test_environment(|env| async move {
        let resp = env.api.get("/auth/protected").await;
        assert_eq!(StatusCode::UNAUTHORIZED, resp.status());
    }).await;
}

#[tokio::test]
async fn test_access_with_expired_token() {
    with_test_environment(|mut env| async move {
        let expires_in = Utc::now() - chrono::Duration::days(1);
        let token = generate_access_token("0", expires_in.timestamp());
        env.api.set_token(token);

        let resp = env.api.get("/auth/protected").await;
        assert_eq!(StatusCode::UNAUTHORIZED, resp.status());
    }).await;
}

