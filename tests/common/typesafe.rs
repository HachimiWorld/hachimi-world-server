use hachimi_world_server::service::content_check::{ContentCheckCfg, ContentChecker};
use hachimi_world_server::util::typesafe::{MockTypeSafeClient, SystemOneResponse, TypeSafeCfg};
use sqlx::PgPool;
use std::sync::Arc;

/// The model name the mock reports for text.
pub const TEST_MODEL: &str = "jev-test";
/// The model name the mock reports for images.
pub const TEST_IMAGE_MODEL: &str = "jev-test-image";

/// A content checker using `config/content-check-example` and [get_mock_typesafe_client], so the
/// example policy is kept loadable too.
pub async fn get_test_content_checker(pool: &PgPool) -> ContentChecker {
    new_content_checker(pool, get_mock_typesafe_client()).await.unwrap()
}

/// A content checker using `config/content-check-example` and `client`.
pub async fn new_content_checker(pool: &PgPool, client: MockTypeSafeClient) -> anyhow::Result<ContentChecker> {
    let cfg = ContentCheckCfg {
        policy_dir: "config/content-check-example".into(),
        model: TEST_MODEL.to_string(),
        image_model: TEST_IMAGE_MODEL.to_string(),
        typesafe: TypeSafeCfg { api_key: String::new(), base_url: String::new(), timeout_ms: 0 },
    };
    ContentChecker::new(&cfg, Arc::new(client), pool).await
}

/// A mock TypeSafe client whose every call fails with what `error` makes.
pub fn get_failing_typesafe_client(error: fn() -> anyhow::Error) -> MockTypeSafeClient {
    let mut mock = MockTypeSafeClient::default();
    mock.expect_system_one().returning(move |_, _, _| {
        let e = error();
        Box::pin(async move { Err(e) })
    });
    mock.expect_system_one_image().returning(move |_, _, _| {
        let e = error();
        Box::pin(async move { Err(e) })
    });
    mock
}

/// A mock TypeSafe client whose answers depend on markers in the text:
/// - `#<question id>`, such as `#sexual`, gives that noul 0.95; other nouls get 0.05
/// - `#down` fails the call
/// - `#odd` returns no answers, as if the response didn't match the policy
///
/// Images are read as the marker for their main colour (see [image_marker]): a mostly red image
/// is `#sexual`, a mostly blue one is `#down`.
pub fn get_mock_typesafe_client() -> MockTypeSafeClient {
    let mut mock = MockTypeSafeClient::default();
    mock.expect_system_one().returning(|model, state, questions| {
        let result = answer(model, state, questions);
        Box::pin(async move { result })
    });
    mock.expect_system_one_image().returning(|model, jpeg, questions| {
        let result = image_marker(jpeg).and_then(|marker| answer(model, marker, questions));
        Box::pin(async move { result })
    });
    mock
}

/// [get_mock_typesafe_client] after TypeSafe came back: `#down` no longer fails the call, and the
/// other markers work as before.
pub fn get_recovered_typesafe_client() -> MockTypeSafeClient {
    let mut mock = MockTypeSafeClient::default();
    mock.expect_system_one().returning(|model, state, questions| {
        let result = answer(model, &state.replace("#down", ""), questions);
        Box::pin(async move { result })
    });
    mock.expect_system_one_image().returning(|model, jpeg, questions| {
        let result = image_marker(jpeg).and_then(|marker| answer(model, &marker.replace("#down", ""), questions));
        Box::pin(async move { result })
    });
    mock
}

/// The marker a mock image stands for, by the average colour of the JPEG sent.
fn image_marker(jpeg: &[u8]) -> anyhow::Result<&'static str> {
    let image = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)?.to_rgb8();
    let mut sum = [0u64; 3];
    for pixel in image.pixels() {
        for (s, c) in sum.iter_mut().zip(pixel.0) {
            *s += c as u64;
        }
    }
    let [r, g, b] = sum.map(|s| s / image.pixels().len() as u64);
    Ok(if r > 180 && g < 80 && b < 80 {
        "#sexual"
    } else if b > 180 && r < 80 && g < 80 {
        "#down"
    } else {
        ""
    })
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
