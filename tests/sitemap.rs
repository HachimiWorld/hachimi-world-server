use crate::common::auth::{with_new_random_test_user, with_test_contributor_user};
use crate::common::publish::create_approved_song;
use crate::common::with_test_environment;
use crate::common::CommonParse;
use hachimi_world_server::web::routes::sitemap::{GenerationListReq, GenerationListResp, SitemapGenerationItem};
use reqwest::StatusCode;

mod common;

const TRIGGER_TOKEN: &str = "test-sitemap-trigger-token"; // From test-config.yaml

#[tokio::test]
async fn test_sitemap_endpoints_require_trigger_token() {
    with_test_environment(|mut env| async move {
        let resp = env.api.post("/sitemap/generate", &()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let resp = env.api.get("/sitemap/generation/list").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // A user's JWT is not the trigger token either
        let user = with_new_random_test_user(&mut env).await;
        env.api.set_token(user.token.access_token.clone());
        let resp = env.api.post("/sitemap/generate", &()).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        env.api.set_token("wrong-token-wrong-token".to_string());
        let resp = env.api.get("/sitemap/generation/list").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }).await;
}

#[tokio::test]
async fn test_generate_sitemap_and_list_generations() {
    with_test_environment(|mut env| async move {
        let owner = with_new_random_test_user(&mut env).await;
        let contributor = with_test_contributor_user(&mut env).await;
        create_approved_song(&mut env, &owner.token.access_token, &contributor.token.access_token, "Sitemap Song").await;

        env.api.set_token(TRIGGER_TOKEN.to_string());
        let generation: SitemapGenerationItem = env.api.post("/sitemap/generate", &()).await
            .parse_resp().await.unwrap();
        assert_eq!(generation.trigger_type, "manual");
        assert_eq!(generation.status, "success", "{:?}", generation.error);
        assert_eq!(generation.song_count, Some(1));
        assert_eq!(generation.file_count, Some(2)); // songs-0.xml and index.xml
        assert!(generation.finish_time.is_some());

        let resp: GenerationListResp = env.api.get_query("/sitemap/generation/list", &GenerationListReq { page_index: 0, page_size: 10 }).await
            .parse_resp().await.unwrap();
        assert_eq!(resp.total, 1);
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].id, generation.id);
        assert_eq!(resp.data[0].status, "success");
    }).await;
}
