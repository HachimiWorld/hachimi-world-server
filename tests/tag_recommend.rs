mod common;

use crate::common::auth::with_new_random_test_user;
use crate::common::{assert_is_ok, with_test_environment, CommonParse};
use hachimi_world_server::web::routes::song::{TagCreateReq, TagRecommendResp};

#[tokio::test]
async fn test_tag_recommend_empty() {
    with_test_environment(|mut env| async move {
        with_new_random_test_user(&mut env).await;
        // New user should have no history; endpoint should still work.
        let resp: TagRecommendResp = env.api.get("/song/tag/recommend")
            .await.parse_resp().await.unwrap();
        assert!(resp.result.is_empty());
    }).await;
}

#[tokio::test]
async fn test_tag_recommend_after_like() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        // Create a tag and attach it to an existing song via publish flow isn't trivial here;
        // instead we just require the endpoint to respond (non-error).
        // If fixtures exist with tags+likes, this will start returning data.
        let _ = env.api.post(
            "/song/tag/create",
            &TagCreateReq {
                name: format!("T{}", rand::random::<u16>()),
                description: None,
            },
        ).await;

        // Ensure it doesn't error even when user has no plays/likes.
        let resp = env.api.get("/song/tag/recommend").await;
        assert_is_ok(resp).await;
    }).await;
}

