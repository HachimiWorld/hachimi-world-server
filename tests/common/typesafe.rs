use hachimi_world_server::service::content_check::{ContentCheckCfg, ContentChecker};
use hachimi_world_server::util::typesafe::{MockTypeSafeClient, SystemOneResponse, TypeSafeCfg};
use sqlx::PgPool;
use std::sync::Arc;

/// The model name the mock reports.
pub const TEST_MODEL: &str = "jev-test";

/// A content checker using `config/content-check-example` and [get_mock_typesafe_client], so the
/// example policy is kept loadable too.
pub async fn get_test_content_checker(pool: &PgPool) -> ContentChecker {
    let cfg = ContentCheckCfg {
        policy_dir: "config/content-check-example".into(),
        model: TEST_MODEL.to_string(),
        typesafe: TypeSafeCfg { api_key: String::new(), base_url: String::new(), timeout_ms: 0 },
    };
    ContentChecker::new(&cfg, Arc::new(get_mock_typesafe_client()), pool).await.unwrap()
}

/// A mock TypeSafe client whose answers depend on markers in the text:
/// - `#<question id>`, such as `#sexual`, gives that noul 0.95; other nouls get 0.05
/// - `#down` fails the call
/// - `#odd` returns no answers, as if the response didn't match the policy
pub fn get_mock_typesafe_client() -> MockTypeSafeClient {
    let mut mock = MockTypeSafeClient::default();
    mock.expect_system_one().returning(|model, state, questions| {
        let result = answer(model, state, questions);
        Box::pin(async move { result })
    });
    mock
}

fn answer(model: &str, state: &str, questions: &serde_json::Value) -> anyhow::Result<SystemOneResponse> {
    if state.contains("#down") {
        anyhow::bail!("TypeSafe returned 503 Service Unavailable: mock");
    }
    let mut answers = serde_json::Map::new();
    if !state.contains("#odd") {
        for id in questions.as_object().unwrap().keys() {
            let p = if state.contains(&format!("#{id}")) { 0.95 } else { 0.05 };
            answers.insert(id.clone(), serde_json::json!({ "type": "noul", "noul": p }));
        }
    }
    Ok(serde_json::from_value(serde_json::json!({
        "model": model,
        "answers": answers,
        "usage": { "input_tokens": 10 },
    }))?)
}
