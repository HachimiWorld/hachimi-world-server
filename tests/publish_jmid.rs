use crate::common::auth::with_new_random_test_user;
use crate::common::publish::publish_template;
use crate::common::with_test_environment;
use crate::common::CommonParse;
use hachimi_world_server::web::routes::publish::jmid::{JmidCheckPReq, JmidCheckPResp, JmidCheckReq};
use hachimi_world_server::web::routes::publish::PublishResp;

mod common;

#[tokio::test]
async fn test_check_jmid_prefix_should_not_available_when_used() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        // Never used prefix should be available
        let resp = env.api.get_query("/publish/jmid/check_prefix", &JmidCheckPReq {
            jmid_prefix: "ABCD".to_string(),
        }).await.parse_resp::<JmidCheckPResp>().await.unwrap();
        assert_eq!(resp.result, true);

        // Publish with this prefix
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-001".to_string());

        let _resp = env.api.post("/publish/publish", &req)
            .await
            .parse_resp::<PublishResp>()
            .await
            .unwrap();

        let resp = env.api.get_query("/publish/jmid/check_prefix", &JmidCheckPReq {
            jmid_prefix: "ABCD".to_string(),
        }).await.parse_resp::<JmidCheckPResp>().await.unwrap();
        assert_eq!(resp.result, false);
    }).await
}

#[tokio::test]
async fn test_check_jmid() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;
        let resp = env.api.get_query("/publish/jmid/check", &JmidCheckReq {
            jmid: "JM-TEST-001".to_string(),
        }).await.parse_resp::<JmidCheckPResp>().await.unwrap();
        assert_eq!(resp.result, true);

        // Publish with this prefix
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-TEST-001".to_string());

        let _resp = env.api.post("/publish/publish", &req)
            .await.parse_resp::<PublishResp>().await.unwrap();

        // Check the same jmid should not be available
        let resp = env.api.get_query("/publish/jmid/check", &JmidCheckReq {
            jmid: "JM-TEST-001".to_string(),
        }).await.parse_resp::<JmidCheckPResp>().await.unwrap();
        assert_eq!(resp.result, false);

        // Check another jmid with same prefix should be available
        let resp = env.api.get_query("/publish/jmid/check", &JmidCheckReq {
            jmid: "JM-TEST-002".to_string(),
        }).await.parse_resp::<JmidCheckPResp>().await.unwrap();
        assert_eq!(resp.result, true);
    }).await
}