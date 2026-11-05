mod common;

use crate::common::auth::{generate_pass_captcha_key, with_new_random_test_user, TestUser};
use crate::common::{CommonParse, TestEnvironment};
use chrono::{Duration, Utc};
use common::with_test_environment;
use hachimi_world_server::db::email_outbox::EmailOutbox;
use hachimi_world_server::service::email_change::{self, AccountCfg};
use hachimi_world_server::service::verification_code::{self, CodeKeys};
use hachimi_world_server::web::routes::auth::{LoginReq, LoginResp, RefreshTokenReq, TokenPair};
use hachimi_world_server::web::routes::email_change::{
    ChangeEmailReq, EmailInfoResp, PendingEmailChange, RequestEmailChangeReq, RevertEmailChangeReq, SendNewEmailCodeReq,
};

const PASSWORD: &str = "test12345678";

fn random_email() -> String {
    format!("new_{}@example.com", uuid::Uuid::new_v4())
}

async fn info(env: &TestEnvironment) -> EmailInfoResp {
    env.api.get("/auth/email/info").await.parse_resp().await.unwrap()
}

async fn login(env: &TestEnvironment, email: &str, password: &str) -> Result<LoginResp, String> {
    let captcha_key = generate_pass_captcha_key(&env.api).await;
    env.api.post("/auth/login/email", &LoginReq {
        email: email.to_string(),
        password: password.to_string(),
        device_info: "test".to_string(),
        code: None,
        captcha_key,
    }).await.parse_resp::<LoginResp>().await.map_err(|e| e.code)
}

async fn set_new_code(env: &mut TestEnvironment, user: &TestUser, new_email: &str) -> String {
    verification_code::set_code_with(&mut env.redis, &CodeKeys::change_email_new(user.uid, new_email), "123456").await.unwrap();
    "123456".to_string()
}

async fn set_old_code(env: &mut TestEnvironment, user: &TestUser) -> String {
    verification_code::set_code_with(&mut env.redis, &CodeKeys::change_email_old(user.uid), "654321").await.unwrap();
    "654321".to_string()
}

async fn request_with_wait(env: &mut TestEnvironment, user: &TestUser, new_email: &str) -> PendingEmailChange {
    let new_code = set_new_code(env, user, new_email).await;
    let pending = env.api.post("/auth/email/request_change", &RequestEmailChangeReq {
        new_email: new_email.to_string(),
        new_code,
        password: PASSWORD.to_string(),
    }).await.parse_resp().await.unwrap();
    mark_due_emails_sent(env).await;
    pending
}

/// The relay doesn't run in tests: act as if it sent the emails that are due.
async fn mark_due_emails_sent(env: &TestEnvironment) {
    sqlx::query("UPDATE email_outbox SET sent_time = NOW() WHERE sent_time IS NULL AND available_time <= NOW()")
        .execute(&env.pool).await.unwrap();
}

async fn emails_to(env: &TestEnvironment, to: &str) -> Vec<EmailOutbox> {
    sqlx::query_as::<_, EmailOutbox>("SELECT * FROM email_outbox WHERE to_address = $1 ORDER BY create_time, available_time")
        .bind(to)
        .fetch_all(&env.pool).await.unwrap()
}

/// The undo token in the last email to `to`.
async fn last_token_to(env: &TestEnvironment, to: &str) -> String {
    let emails = emails_to(env, to).await;
    let body = &emails.last().expect("no email").body;
    let start = body.find("token=").expect("no link") + "token=".len();
    body[start..start + 64].to_string()
}

#[tokio::test]
async fn test_change_with_old_email() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;
        let new_email = random_email();

        // Codes are checked
        let old_code = set_old_code(&mut env, &user).await;
        let new_code = set_new_code(&mut env, &user, &new_email).await;
        let err = env.api.post("/auth/email/change", &ChangeEmailReq {
            new_email: new_email.clone(),
            old_code: "000000".to_string(),
            new_code: new_code.clone(),
        }).await.parse_resp::<()>().await.unwrap_err();
        assert_eq!(err.code, "invalid_old_code");

        // A code for another purpose doesn't work
        verification_code::set_code(&mut env.redis, &new_email, "111111").await.unwrap();
        let err = env.api.post("/auth/email/change", &ChangeEmailReq {
            new_email: new_email.clone(),
            old_code: old_code.clone(),
            new_code: "111111".to_string(),
        }).await.parse_resp::<()>().await.unwrap_err();
        assert_eq!(err.code, "invalid_new_code");

        let old_code = set_old_code(&mut env, &user).await;
        let new_code = set_new_code(&mut env, &user, &new_email).await;
        env.api.post("/auth/email/change", &ChangeEmailReq {
            new_email: new_email.clone(),
            old_code,
            new_code,
        }).await.parse_resp::<()>().await.unwrap();

        assert_eq!(info(&env).await.email, new_email);
        assert!(login(&env, &new_email, PASSWORD).await.is_ok());
        assert!(login(&env, &user.email, PASSWORD).await.is_err());

        // The old address is told
        let emails = emails_to(&env, &user.email).await;
        assert_eq!(emails.len(), 1);
        assert!(!emails[0].body.contains(&new_email), "The new email is masked");
    }).await;
}

#[tokio::test]
async fn test_new_email_checks() {
    with_test_environment(|mut env| async move {
        let other = with_new_random_test_user(&mut env).await;
        let user = with_new_random_test_user(&mut env).await;

        for (email, code) in [
            ("not-an-email", "invalid_email"),
            (user.email.as_str(), "same_email"),
            (other.email.as_str(), "email_existed"),
        ] {
            let err = env.api.post("/auth/email/send_new_code", &SendNewEmailCodeReq { new_email: email.to_string() })
                .await.parse_resp::<()>().await.unwrap_err();
            assert_eq!(err.code, code);
        }

        // Sending is limited per user, not per address
        env.api.post("/auth/email/send_new_code", &SendNewEmailCodeReq { new_email: random_email() })
            .await.parse_resp::<()>().await.unwrap();
        let err = env.api.post("/auth/email/send_new_code", &SendNewEmailCodeReq { new_email: random_email() })
            .await.parse_resp::<()>().await.unwrap_err();
        assert_eq!(err.code, "too_many_requests");

        env.api.post("/auth/email/send_old_code", &()).await.parse_resp::<()>().await.unwrap();
    }).await;
}

#[tokio::test]
async fn test_request_with_wait_applies_after_wait() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;
        let new_email = random_email();

        // Wrong password
        let new_code = set_new_code(&mut env, &user, &new_email).await;
        let err = env.api.post("/auth/email/request_change", &RequestEmailChangeReq {
            new_email: new_email.clone(),
            new_code,
            password: "wrong-password".to_string(),
        }).await.parse_resp::<PendingEmailChange>().await.unwrap_err();
        assert_eq!(err.code, "password_not_match");

        let pending = request_with_wait(&mut env, &user, &new_email).await;
        assert_eq!(pending.new_email, new_email);
        assert_eq!(pending.effective_time - pending.create_time, email_change::WAIT);

        let info_resp = info(&env).await;
        assert_eq!(info_resp.email, user.email, "Not changed yet");
        assert_eq!(info_resp.pending.unwrap().new_email, new_email);
        assert!(info_resp.next_request_time.is_some());

        // Three reminders to the old address, spread over the wait
        let reminders = emails_to(&env, &user.email).await;
        assert_eq!(reminders.len(), 3);
        assert!(reminders[1].available_time > Utc::now() + Duration::days(2));
        assert!(reminders.iter().all(|x| x.body.contains("token=")));

        // Only one at a time
        let new_code = set_new_code(&mut env, &user, &new_email).await;
        let err = env.api.post("/auth/email/request_change", &RequestEmailChangeReq {
            new_email: new_email.clone(),
            new_code,
            password: PASSWORD.to_string(),
        }).await.parse_resp::<PendingEmailChange>().await.unwrap_err();
        assert_eq!(err.code, "pending_exists");

        // Not due yet
        let cfg = AccountCfg::default();
        email_change::apply_due(&env.pool, &cfg, Utc::now()).await.unwrap();
        assert_eq!(info(&env).await.email, user.email);

        email_change::apply_due(&env.pool, &cfg, Utc::now() + email_change::WAIT + Duration::minutes(1)).await.unwrap();
        let info_resp = info(&env).await;
        assert_eq!(info_resp.email, new_email);
        assert!(info_resp.pending.is_none());
        assert!(login(&env, &new_email, PASSWORD).await.is_ok());

        // Reminders not sent yet are dropped, and the old address gets a notice
        let emails = emails_to(&env, &user.email).await;
        assert_eq!(emails.len(), 2, "{emails:?}");
        assert!(emails[1].subject.contains("已更换"));

        // Once in 30 days
        let other_email = random_email();
        let new_code = set_new_code(&mut env, &user, &other_email).await;
        let err = env.api.post("/auth/email/request_change", &RequestEmailChangeReq {
            new_email: other_email,
            new_code,
            password: PASSWORD.to_string(),
        }).await.parse_resp::<PendingEmailChange>().await.unwrap_err();
        assert_eq!(err.code, "too_frequent");
    }).await;
}

#[tokio::test]
async fn test_cancel_pending() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;
        let new_email = random_email();
        request_with_wait(&mut env, &user, &new_email).await;

        env.api.post("/auth/email/cancel_change", &()).await.parse_resp::<()>().await.unwrap();
        let info_resp = info(&env).await;
        assert!(info_resp.pending.is_none());
        assert!(info_resp.next_request_time.is_none(), "A cancelled request doesn't count");
        assert_eq!(emails_to(&env, &user.email).await.len(), 1, "Only the reminder already due is kept");

        email_change::apply_due(&env.pool, &AccountCfg::default(), Utc::now() + Duration::days(8)).await.unwrap();
        assert_eq!(info(&env).await.email, user.email);
    }).await;
}

#[tokio::test]
async fn test_revert_pending_from_old_email() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;
        let new_email = random_email();
        request_with_wait(&mut env, &user, &new_email).await;
        let token = last_token_to(&env, &user.email).await;

        let err = env.api.post("/auth/email/revert_change", &RevertEmailChangeReq {
            token: token.clone(),
            new_password: "short".to_string(),
        }).await.parse_resp::<()>().await.unwrap_err();
        assert_eq!(err.code, "invalid_password");

        env.api.clear_token();
        env.api.post("/auth/email/revert_change", &RevertEmailChangeReq {
            token: token.clone(),
            new_password: "new-password-1".to_string(),
        }).await.parse_resp::<()>().await.unwrap();

        // Every device is logged out, and the new password is needed
        let err = env.api.post("/auth/refresh_token", &RefreshTokenReq {
            refresh_token: user.token.refresh_token.clone(),
            device_info: "test".to_string(),
        }).await.parse_resp::<TokenPair>().await.unwrap_err();
        assert_eq!(err.code, "token_not_found");
        assert!(login(&env, &user.email, PASSWORD).await.is_err());
        let resp = login(&env, &user.email, "new-password-1").await.unwrap();

        // The change never applies
        email_change::apply_due(&env.pool, &AccountCfg::default(), Utc::now() + Duration::days(8)).await.unwrap();
        env.api.set_token(resp.token.access_token);
        assert_eq!(info(&env).await.email, user.email);

        // The link works once
        let err = env.api.post("/auth/email/revert_change", &RevertEmailChangeReq {
            token,
            new_password: "new-password-2".to_string(),
        }).await.parse_resp::<()>().await.unwrap_err();
        assert_eq!(err.code, "invalid_token");
    }).await;
}

#[tokio::test]
async fn test_revert_applied_from_old_email() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;
        let new_email = random_email();
        request_with_wait(&mut env, &user, &new_email).await;
        let reminder_token = last_token_to(&env, &user.email).await;
        email_change::apply_due(&env.pool, &AccountCfg::default(), Utc::now() + Duration::days(8)).await.unwrap();

        // The link in the reminders stops working once applied; the notice has a new one
        let err = env.api.post("/auth/email/revert_change", &RevertEmailChangeReq {
            token: reminder_token,
            new_password: "new-password-1".to_string(),
        }).await.parse_resp::<()>().await.unwrap_err();
        assert_eq!(err.code, "invalid_token");

        // Even after another change made at once from the new address
        let newer_email = random_email();
        let old_code = set_old_code(&mut env, &user).await;
        let new_code = set_new_code(&mut env, &user, &newer_email).await;
        env.api.post("/auth/email/change", &ChangeEmailReq {
            new_email: newer_email.clone(),
            old_code,
            new_code,
        }).await.parse_resp::<()>().await.unwrap();

        let token = last_token_to(&env, &user.email).await;
        env.api.post("/auth/email/revert_change", &RevertEmailChangeReq {
            token,
            new_password: "new-password-1".to_string(),
        }).await.parse_resp::<()>().await.unwrap();

        assert!(login(&env, &newer_email, "new-password-1").await.is_err());
        assert!(login(&env, &new_email, "new-password-1").await.is_err());
        let resp = login(&env, &user.email, "new-password-1").await.unwrap();
        env.api.set_token(resp.token.access_token);
        assert_eq!(info(&env).await.email, user.email);
    }).await;
}

#[tokio::test]
async fn test_too_many_wrong_passwords() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;
        let new_email = random_email();
        for _ in 0..5 {
            let new_code = set_new_code(&mut env, &user, &new_email).await;
            let err = env.api.post("/auth/email/request_change", &RequestEmailChangeReq {
                new_email: new_email.clone(),
                new_code,
                password: "wrong-password".to_string(),
            }).await.parse_resp::<PendingEmailChange>().await.unwrap_err();
            assert_eq!(err.code, "password_not_match");
        }
        let new_code = set_new_code(&mut env, &user, &new_email).await;
        let err = env.api.post("/auth/email/request_change", &RequestEmailChangeReq {
            new_email: new_email.clone(),
            new_code,
            password: PASSWORD.to_string(),
        }).await.parse_resp::<PendingEmailChange>().await.unwrap_err();
        assert_eq!(err.code, "too_many_attempts");
    }).await;
}
