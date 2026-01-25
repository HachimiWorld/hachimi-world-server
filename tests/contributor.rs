use crate::common::auth::{with_new_random_test_user, with_test_contributor_user};
use crate::common::with_test_environment;
use crate::common::CommonParse;
use hachimi_world_server::web::routes::contributor::CheckContributorResp;

mod common;

#[tokio::test]
async fn test_check_contributor() {
    with_test_environment(|mut env| async move {
        let _test_user = with_new_random_test_user(&mut env).await;

        let resp: CheckContributorResp = env.api
            .get("/contributor/check").await
            .parse_resp().await.unwrap();

        assert_eq!(resp.is_contributor, false);

        let _test_cont_user = with_test_contributor_user(&mut env).await;

        let resp: CheckContributorResp = env.api
            .get("/contributor/check").await
            .parse_resp().await.unwrap();
        assert_eq!(resp.is_contributor, true);
    }).await;
}