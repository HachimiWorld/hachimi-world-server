use hachimi_world_server::web::routes::song::TagCreateReq;
use crate::common::{assert_is_ok, ApiClient};

pub async fn create_tags(api: &ApiClient) {
    // TODO[test](song): We should add cleanup code to make the test repeatable.
    let tags = vec!["原教旨", "流行", "古典", "人声翻唱", "摇滚", "R&B", "民谣"];
    for x in tags {
        let resp = api
            .post(
                "/song/tag/create",
                &TagCreateReq {
                    name: x.to_string(),
                    description: None,
                },
            )
            .await;
        assert_is_ok(resp).await;
    }
}
