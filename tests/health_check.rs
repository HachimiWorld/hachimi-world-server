use crate::common::with_test_environment;

mod common;

#[tokio::test]
async fn test_health_check() {
    with_test_environment(|env| async move {
        let response = env.api.get("/health").await;
        assert_eq!(200, response.status());
    }).await;
}