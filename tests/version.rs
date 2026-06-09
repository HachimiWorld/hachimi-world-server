use crate::common::with_test_environment;
use crate::common::CommonParse;
use chrono::{Duration, Utc};
use hachimi_world_server::web::routes::version::{
    DeleteVersionReq, LatestVersionBatchReq, LatestVersionReq, LatestVersionResp, PageVersionsReq,
    PageVersionsResp, PublishVersionReq, PublishVersionResp,
};
use itertools::Itertools;
use reqwest::StatusCode;
use serde_json::Value;

mod common;

/// Truncate DateTime to microsecond precision to match JSON serialization round-trip.
fn truncate_to_micros(dt: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    chrono::DateTime::from_timestamp_micros(dt.timestamp_micros()).unwrap()
}

#[tokio::test]
async fn test_publish_version() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string()); // From test-config.yaml
        let now = Utc::now();
        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "test".to_string(),
            variant: "test-android".to_string(),
            url: "https://test.example.com/android/latest.apk".to_string(),
            release_time: now,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        let resp = env.api.get_query("/version/latest", &LatestVersionReq {
            variant: "test-android".to_string(),
        }).await.parse_resp::<Option<LatestVersionResp>>().await.unwrap().unwrap();
        assert_eq!(resp.version_name, "v1.0.0");
        assert_eq!(resp.version_number, 1);
        assert_eq!(resp.changelog, "test");
        assert_eq!(resp.variant, "test-android");
        assert_eq!(resp.url, "https://test.example.com/android/latest.apk");
        assert_eq!(resp.release_time, truncate_to_micros(now));
    }).await
}

#[tokio::test]
async fn test_publish_version_without_token_should_fail() {
    with_test_environment(|env| async move {
        let now = Utc::now();
        let resp = env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "test".to_string(),
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

#[tokio::test]
async fn test_get_server_version_contract() {
    with_test_environment(|env| async move {
        let resp = env.api.get("/version/server").await.parse_resp::<Value>().await.unwrap();
        assert_eq!(resp["version"], 260407);
        assert_eq!(resp["min_version"], 250905);
    }).await
}

#[tokio::test]
async fn test_get_latest_version_returns_none_when_variant_missing() {
    with_test_environment(|env| async move {
        let resp = env.api.get_query("/version/latest", &LatestVersionReq {
            variant: "missing-variant".to_string(),
        }).await.parse_resp::<Option<LatestVersionResp>>().await.unwrap();
        assert!(resp.is_none());
    }).await
}

#[tokio::test]
async fn test_get_latest_version_returns_latest_released_version_for_same_variant() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let first_release = Utc::now() - Duration::hours(2);
        let latest_release = Utc::now() - Duration::hours(1);

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "first".to_string(),
            variant: "release-android".to_string(),
            url: "https://test.example.com/android/v1.apk".to_string(),
            release_time: first_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.1.0".to_string(),
            version_number: 2,
            changelog: "latest".to_string(),
            variant: "release-android".to_string(),
            url: "https://test.example.com/android/v2.apk".to_string(),
            release_time: latest_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        let resp = env.api.get_query("/version/latest", &LatestVersionReq {
            variant: "release-android".to_string(),
        }).await.parse_resp::<Option<LatestVersionResp>>().await.unwrap().unwrap();

        assert_eq!(resp.version_name, "v1.1.0");
        assert_eq!(resp.version_number, 2);
        assert_eq!(resp.changelog, "latest");
        assert_eq!(resp.url, "https://test.example.com/android/v2.apk");
        assert_eq!(resp.release_time, truncate_to_micros(latest_release));
    }).await
}

#[tokio::test]
async fn test_get_latest_version_skips_future_release() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let released_at = Utc::now() - Duration::hours(1);
        let future_release = Utc::now() + Duration::hours(1);

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "released".to_string(),
            variant: "release-android".to_string(),
            url: "https://test.example.com/android/v1.apk".to_string(),
            release_time: released_at,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v2.0.0".to_string(),
            version_number: 2,
            changelog: "future".to_string(),
            variant: "release-android".to_string(),
            url: "https://test.example.com/android/v2.apk".to_string(),
            release_time: future_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        let resp = env.api.get_query("/version/latest", &LatestVersionReq {
            variant: "release-android".to_string(),
        }).await.parse_resp::<Option<LatestVersionResp>>().await.unwrap().unwrap();

        assert_eq!(resp.version_name, "v1.0.0");
        assert_eq!(resp.version_number, 1);
        assert_eq!(resp.release_time, truncate_to_micros(released_at));
    }).await
}

#[tokio::test]
async fn test_publish_version_clears_latest_cache() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let first_release = Utc::now() - Duration::hours(2);
        let latest_release = Utc::now() - Duration::hours(1);

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "first".to_string(),
            variant: "release-android".to_string(),
            url: "https://test.example.com/android/v1.apk".to_string(),
            release_time: first_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        let cached = env.api.get_query("/version/latest", &LatestVersionReq {
            variant: "release-android".to_string(),
        }).await.parse_resp::<Option<LatestVersionResp>>().await.unwrap().unwrap();
        assert_eq!(cached.version_number, 1);

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.1.0".to_string(),
            version_number: 2,
            changelog: "latest".to_string(),
            variant: "release-android".to_string(),
            url: "https://test.example.com/android/v2.apk".to_string(),
            release_time: latest_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        let refreshed = env.api.get_query("/version/latest", &LatestVersionReq {
            variant: "release-android".to_string(),
        }).await.parse_resp::<Option<LatestVersionResp>>().await.unwrap().unwrap();
        assert_eq!(refreshed.version_name, "v1.1.0");
        assert_eq!(refreshed.version_number, 2);
        assert_eq!(refreshed.release_time, truncate_to_micros(latest_release));
    }).await
}

#[tokio::test]
async fn test_get_version_batch_ignores_missing_variants() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let now = Utc::now() - Duration::hours(1);

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: now,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        let result = env.api.post("/version/latest_batch", &LatestVersionBatchReq {
            variants: vec!["release-android".to_string(), "release-ios".to_string()],
        }).await.parse_resp::<Vec<LatestVersionResp>>().await.unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].variant, "release-android");
    }).await
}

#[tokio::test]
async fn test_get_version_batch_rejects_more_than_16_variants() {
    with_test_environment(|env| async move {
        let err = env.api.post("/version/latest_batch", &LatestVersionBatchReq {
            variants: (0..17).map(|i| format!("variant-{i}")).collect_vec(),
        }).await.parse_resp::<Vec<LatestVersionResp>>().await.unwrap_err();

        assert_eq!(err.code, "bad_request");
        assert_eq!(err.msg, "Variants must be less than 16");
    }).await
}

#[tokio::test]
async fn test_get_version_batch_applies_latest_selection_rules() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let old_release = Utc::now() - Duration::hours(3);
        let latest_release = Utc::now() - Duration::hours(1);
        let future_release = Utc::now() + Duration::hours(1);

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: old_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v2.0.0".to_string(),
            version_number: 2,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: future_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 10,
            changelog: "".to_string(),
            variant: "release-macos".to_string(),
            url: "".to_string(),
            release_time: old_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.1.0".to_string(),
            version_number: 11,
            changelog: "".to_string(),
            variant: "release-macos".to_string(),
            url: "".to_string(),
            release_time: latest_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        let result = env.api.post("/version/latest_batch", &LatestVersionBatchReq {
            variants: vec!["release-android".to_string(), "release-macos".to_string()],
        }).await.parse_resp::<Vec<LatestVersionResp>>().await.unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result.iter().find(|v| v.variant == "release-android").unwrap().version_number, 1);
        assert_eq!(result.iter().find(|v| v.variant == "release-macos").unwrap().version_number, 11);
    }).await
}

#[tokio::test]
async fn test_page_versions_returns_desc_order_and_total() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let first_release = Utc::now() - Duration::hours(3);
        let second_release = Utc::now() - Duration::hours(2);
        let third_release = Utc::now() - Duration::hours(1);

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: first_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.1.0".to_string(),
            version_number: 2,
            changelog: "".to_string(),
            variant: "release-ios".to_string(),
            url: "".to_string(),
            release_time: second_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.2.0".to_string(),
            version_number: 3,
            changelog: "".to_string(),
            variant: "release-macos".to_string(),
            url: "".to_string(),
            release_time: third_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        let page = env.api.get_query("/version/page", &PageVersionsReq {
            variant: None,
            page_index: 0,
            page_size: 10,
        }).await.parse_resp::<PageVersionsResp>().await.unwrap();

        assert_eq!(page.total, 3);
        assert_eq!(page.data.len(), 3);
        assert_eq!(page.data[0].version_name, "v1.2.0");
        assert_eq!(page.data[1].version_name, "v1.1.0");
        assert_eq!(page.data[2].version_name, "v1.0.0");
    }).await
}

#[tokio::test]
async fn test_page_versions_filters_by_variant() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let first_release = Utc::now() - Duration::hours(3);
        let second_release = Utc::now() - Duration::hours(2);
        let third_release = Utc::now() - Duration::hours(1);

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: first_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.1.0".to_string(),
            version_number: 2,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: third_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 3,
            changelog: "".to_string(),
            variant: "release-ios".to_string(),
            url: "".to_string(),
            release_time: second_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        let page = env.api.get_query("/version/page", &PageVersionsReq {
            variant: Some("release-android".to_string()),
            page_index: 0,
            page_size: 10,
        }).await.parse_resp::<PageVersionsResp>().await.unwrap();

        assert_eq!(page.total, 2);
        assert_eq!(page.data.len(), 2);
        assert!(page.data.iter().all(|item| item.variant == "release-android"));
        assert_eq!(page.data[0].version_number, 2);
        assert_eq!(page.data[1].version_number, 1);
    }).await
}

#[tokio::test]
async fn test_page_versions_clamps_page_index_and_page_size() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let first_release = Utc::now() - Duration::hours(2);
        let second_release = Utc::now() - Duration::hours(1);

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: first_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.1.0".to_string(),
            version_number: 2,
            changelog: "".to_string(),
            variant: "release-ios".to_string(),
            url: "".to_string(),
            release_time: second_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        let min_page = env.api.get_query("/version/page", &PageVersionsReq {
            variant: None,
            page_index: -1,
            page_size: 0,
        }).await.parse_resp::<PageVersionsResp>().await.unwrap();
        assert_eq!(min_page.page_index, 0);
        assert_eq!(min_page.page_size, 1);
        assert_eq!(min_page.data.len(), 1);

        let max_page = env.api.get_query("/version/page", &PageVersionsReq {
            variant: None,
            page_index: 0,
            page_size: 100,
        }).await.parse_resp::<PageVersionsResp>().await.unwrap();
        assert_eq!(max_page.page_index, 0);
        assert_eq!(max_page.page_size, 50);
        assert_eq!(max_page.total, 2);
        assert_eq!(max_page.data.len(), 2);
    }).await
}

#[tokio::test]
async fn test_delete_version_without_token_should_fail() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let id = env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: Utc::now() - Duration::hours(1),
        }).await.parse_resp::<PublishVersionResp>().await.unwrap().id;

        env.api.clear_token();
        let resp = env.api.post("/version/delete", &DeleteVersionReq { id }).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }).await
}

#[tokio::test]
async fn test_delete_version_removes_data_from_latest_batch_and_page() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let first_release = Utc::now() - Duration::hours(2);
        let second_release = Utc::now() - Duration::hours(1);

        let deleted_id = env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: first_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap().id;
        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 2,
            changelog: "".to_string(),
            variant: "release-ios".to_string(),
            url: "".to_string(),
            release_time: second_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();

        env.api.post("/version/delete", &DeleteVersionReq {
            id: deleted_id,
        }).await.parse_resp::<()>().await.unwrap();

        let latest = env.api.get_query("/version/latest", &LatestVersionReq {
            variant: "release-android".to_string(),
        }).await.parse_resp::<Option<LatestVersionResp>>().await.unwrap();
        assert!(latest.is_none());

        let batch = env.api.post("/version/latest_batch", &LatestVersionBatchReq {
            variants: vec!["release-android".to_string(), "release-ios".to_string()],
        }).await.parse_resp::<Vec<LatestVersionResp>>().await.unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].variant, "release-ios");

        let page = env.api.get_query("/version/page", &PageVersionsReq {
            variant: None,
            page_index: 0,
            page_size: 10,
        }).await.parse_resp::<PageVersionsResp>().await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.data.len(), 1);
        assert_eq!(page.data[0].variant, "release-ios");
    }).await
}

#[tokio::test]
async fn test_delete_version_clears_latest_cache_and_falls_back() {
    with_test_environment(|mut env| async move {
        env.api.set_token("12345678".to_string());
        let old_release = Utc::now() - Duration::hours(2);
        let latest_release = Utc::now() - Duration::hours(1);

        env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.0.0".to_string(),
            version_number: 1,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: old_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap();
        let latest_id = env.api.post("/version/publish", &PublishVersionReq {
            version_name: "v1.1.0".to_string(),
            version_number: 2,
            changelog: "".to_string(),
            variant: "release-android".to_string(),
            url: "".to_string(),
            release_time: latest_release,
        }).await.parse_resp::<PublishVersionResp>().await.unwrap().id;

        let cached = env.api.get_query("/version/latest", &LatestVersionReq {
            variant: "release-android".to_string(),
        }).await.parse_resp::<Option<LatestVersionResp>>().await.unwrap().unwrap();
        assert_eq!(cached.version_number, 2);

        env.api.post("/version/delete", &DeleteVersionReq {
            id: latest_id,
        }).await.parse_resp::<()>().await.unwrap();

        let fallback = env.api.get_query("/version/latest", &LatestVersionReq {
            variant: "release-android".to_string(),
        }).await.parse_resp::<Option<LatestVersionResp>>().await.unwrap().unwrap();
        assert_eq!(fallback.version_name, "v1.0.0");
        assert_eq!(fallback.version_number, 1);
        assert_eq!(fallback.release_time, truncate_to_micros(old_release));
    }).await
}
