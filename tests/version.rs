use crate::common::with_test_environment;
use crate::common::CommonParse;
use chrono::Utc;
use hachimi_world_server::web::routes::version::{LatestVersionBatchReq, LatestVersionReq, LatestVersionResp, PublishVersionReq, PublishVersionResp};
use itertools::Itertools;
use reqwest::StatusCode;

mod common;

#[tokio::test]
async fn test_publish_version() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string()); // From test-config.yaml
        let now = Utc::now();
        let resp = env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
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
    }).await
}

#[tokio::test]
async fn test_publish_version_without_token_should_fail() {
    with_test_environment(|mut env| async move {
        let now = Utc::now();
        let resp = env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "Nothing changed".to_string(),
            variant: "test-android".to_string(),
            url: "https://test.example.com/android/latest.apk".to_string(),
            release_time: now,
        }).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED)
    }).await
}
#[tokio::test]
async fn test_get_version_batch() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string()); // From test-config.yaml
        let now = Utc::now();
        let variants = ["release-android", "release-macos", "release-windows"].map(|s| s.to_string()).into_iter().collect_vec();

        for variant in &variants {
            let _ = env.api.post("/version/publish", &PublishVersionReq {
                version_name: "v1.0.0".to_string(),
                version_number: 1,
                changelog: "".to_string(),
                variant: variant.clone(),
                url: "".to_string(),
                release_time: now,
            }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        }

        let result = env.api.post("/version/latest_batch", &LatestVersionBatchReq {
            variants,
        }).await.parse_resp::<Vec<LatestVersionResp>>().await.unwrap();

        assert_eq!(result.len(), 3);
        assert!(result.iter().any(|v| v.variant == "release-android"));
        assert!(result.iter().any(|v| v.variant == "release-macos"));
        assert!(result.iter().any(|v| v.variant == "release-windows"));
    }).await
}