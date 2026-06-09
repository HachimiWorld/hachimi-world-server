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
        for (item1, item2) in resp1.result.iter().zip(resp2.result.iter()) {
            assert_eq!(item1.name, item2.name);
            assert_eq!(item1.description, item2.description);
            assert_ne!(item1.id, item2.id); // The id should be different because the order is different
        }

        // One day later, TODO: We should find a way to mock time
        // Clock.advance(std::time::Duration::from_secs(24 * 3600)).await;
        // let resp3: TagRecommendResp = env.api.get("/song/tag/recommend").await.parse_resp().await.unwrap();
        // assert_ne!(resp1.result.iter().map(|x| x.id).collect::<Vec<_>>(), resp3.result.iter().map(|x| x.id).collect::<Vec<_>>());
    }).await;
}