use crate::common::auth::with_new_random_test_user;
use crate::common::CommonParse;
use crate::common::{assert_is_ok, bilibili, with_test_environment};
use hachimi_world_server::web::routes::user::{ConnectionResp, ConnectionUnlinkReq, GenerateChallengeReq, GenerateChallengeResp, VerifyChallengeReq};
use tokio::time::{sleep, Duration};

mod common;

#[tokio::test]
async fn test_bind_bilibili() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        let generate_challenge_resp = env.api.post("/user/connection/generate_challenge", &GenerateChallengeReq {
            r#type: "bilibili".to_string(),
            provider_account_id: "123456".to_string(),
        }).await.parse_resp::<GenerateChallengeResp>().await.unwrap();
        assert!(!generate_challenge_resp.challenge.is_empty(), "challenge should not be empty");
        assert!(!generate_challenge_resp.challenge_id.is_empty(), "challenge_id should not be empty");
        assert_eq!("测试用户", generate_challenge_resp.provider_account_name);

        // Should fail with wrong challenge
        let challenge_id = generate_challenge_resp.challenge_id;
        let resp = env.api.post("/user/connection/verify_challenge", &VerifyChallengeReq {
            challenge_id: challenge_id.clone(),
        }).await.parse_resp::<()>().await.unwrap_err();
        assert_eq!("challenge_mismatch", resp.code);

        // Wait for redlock to be released asynchronously
        sleep(Duration::from_millis(100)).await;

        // Set bio for mock object
        bilibili::set_mock_test_user_bio(format!("Some bio 123456 {}", generate_challenge_resp.challenge));

        // Should succeed with correct challenge
        let resp = env.api.post("/user/connection/verify_challenge", &VerifyChallengeReq {
            challenge_id,
        }).await;
        assert_is_ok(resp).await;

        // Test list connections
        let connection_resp = env.api.get("/user/connection/list").await.parse_resp::<ConnectionResp>().await.unwrap();
        assert_eq!(connection_resp.items.len(), 1);
        let item = &connection_resp.items[0];
        assert_eq!("bilibili", item.r#type);
        assert_eq!("123456", item.id);
        assert_eq!("测试用户", item.name);

        // Test unlink
        let resp = env.api.post("/user/connection/unlink", &ConnectionUnlinkReq {
            r#type: "bilibili".to_string(),
        }).await;
        assert_is_ok(resp).await;
        let connection_resp = env.api.get("/user/connection/list").await.parse_resp::<ConnectionResp>().await.unwrap();
        assert!(connection_resp.items.is_empty(), "connections should be empty after unlink");
    }).await
}

#[tokio::test]
async fn test_bind_bilibili_should_error_with_invalid_provider_account_id() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        let resp = env.api.post("/user/connection/generate_challenge", &GenerateChallengeReq {
            r#type: "bilibili".to_string(),
            provider_account_id: "000000".to_string(),
        }).await.parse_resp::<GenerateChallengeResp>().await;
        let err = resp.unwrap_err();

        assert_eq!("provider_account_not_found", err.code)
    }).await
}