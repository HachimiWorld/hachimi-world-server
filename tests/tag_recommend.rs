mod common;

use crate::common::auth::{with_new_random_test_user, with_test_contributor_user};
use crate::common::{with_test_environment, CommonParse};
use hachimi_world_server::web::routes::song::{TagCreateReq, TagRecommendResp};

#[tokio::test]
async fn test_tag_recommend() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;
        common::song::create_tags(&env.api).await;

        let _ = env.api.post(
            "/song/tag/create",
            &TagCreateReq {
                name: format!("T{}", rand::random::<u16>()),
                description: None,
            },
        ).await;

        let resp = env.api.get("/song/tag/recommend").await.parse_resp::<TagRecommendResp>().await.unwrap();
        assert!(!resp.result.is_empty());
    }).await;
}

#[tokio::test]
async fn test_tag_recommend_anonymous() {
    with_test_environment(|mut env| async move {
        with_test_contributor_user(&mut env).await;
        common::song::create_tags(&env.api).await;

        env.api.clear_token();

        let resp: TagRecommendResp = env.api.get("/song/tag/recommend_anonymous")
            .await.parse_resp().await.unwrap();
        assert!(!resp.result.is_empty());
    }).await;
}

#[tokio::test]
async fn test_tag_recommend_should_change_everyday() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;
        common::song::create_tags(&env.api).await;

        let resp1: TagRecommendResp = env.api.get("/song/tag/recommend").await.parse_resp().await.unwrap();
        let resp2: TagRecommendResp = env.api.get("/song/tag/recommend").await.parse_resp().await.unwrap();
        assert_eq!(resp1.result.len(), resp2.result.len());
        // Same tags should appear in both responses (order may vary)
        let names1: std::collections::HashSet<_> = resp1.result.iter().map(|x| &x.name).collect();
        let names2: std::collections::HashSet<_> = resp2.result.iter().map(|x| &x.name).collect();
        assert_eq!(names1, names2);
    }).await;
}