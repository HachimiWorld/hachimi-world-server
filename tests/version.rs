use crate::common::with_test_environment;
use crate::common::CommonParse;
use chrono::Utc;
use hachimi_world_server::web::routes::version::{LatestVersionBatchReq, LatestVersionReq, LatestVersionResp, PublishVersionReq, PublishVersionResp};
use std::env;

mod common;

#[tokio::test]
async fn test_publish_version() {
    with_test_environment(|mut env| async move {
        env.api.set_token(env::var("TEST_PUBLISH_VERSION_TOKEN").unwrap());
        let now = Utc::now();
        let resp = env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0-test1".to_string(),
            version_number: 1,
            changelog: "Nothing changed".to_string(),
            variant: "test-android".to_string(),
            url: "https://test.example.com/android/latest.apk".to_string(),
            release_time: now,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        let id = resp.id;
        let resp = env.api.get_query("/version/latest", &LatestVersionReq {
            variant: "test-android".to_string(),
        }).await.parse_resp::<LatestVersionResp>().await.unwrap();
        assert_eq!(resp.version_name, "v1.0.0-test1");
        assert_eq!(resp.version_number, 1);
        assert_eq!(resp.changelog, "Nothing changed");
        assert_eq!(resp.variant, "test-android");
        assert_eq!(resp.url, "https://test.example.com/android/latest.apk");
        assert_eq!(resp.release_time, now);
        ()
    }).await
}

#[tokio::test]
async fn test_get_version_batch() {
    with_test_environment(|mut env| async move {
        let result = env.api.post("/version/latest_batch", &LatestVersionBatchReq{
            variants: vec!["dev-windows".to_string(), "dev-macos".to_string()],
        }).await.parse_resp::<Vec<LatestVersionResp>>().await.unwrap();
        println!("{:?}", result);
    }).await
}