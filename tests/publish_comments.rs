use crate::common::auth::{with_new_random_test_user, with_test_contributor_user};
use crate::common::publish::publish_template;
use crate::common::CommonParse;
use crate::common::{assert_is_ok, with_test_environment};
use hachimi_world_server::web::routes::publish::review::{ReviewCommentCreateReq, ReviewCommentDeleteReq, ReviewCommentListReq, ReviewCommentListResp};
use hachimi_world_server::web::routes::publish::PublishResp;

mod common;

#[tokio::test]
async fn test_create_review_comments_then_delete() {
    with_test_environment(|mut env| async move {
        let uploader = with_new_random_test_user(&mut env).await;
        let publish_resp: PublishResp = env.api.post("/publish/publish", &publish_template(&env).await)
            .await
            .parse_resp()
            .await
            .unwrap();

        // Comment by maintainer
        let maintainer = with_test_contributor_user(&mut env).await;
        let maintainer_comment = "Maintainer comment for testing".to_string();
        let resp = env.api.post("/publish/review/comment/create", &ReviewCommentCreateReq {
            review_id: publish_resp.review_id,
            content: maintainer_comment.clone(),
        }).await;
        assert_is_ok(resp).await;

        // Comment by uploader
        env.api.set_token(uploader.token.access_token.clone());
        let uploader_comment = "Uploader reply for testing".to_string();
        let resp = env.api.post("/publish/review/comment/create", &ReviewCommentCreateReq {
            review_id: publish_resp.review_id,
            content: uploader_comment.clone(),
        }).await;
        assert_is_ok(resp).await;

        // List comments and check
        let resp: ReviewCommentListResp = env.api.get_query("/publish/review/comment/list", &ReviewCommentListReq {
            review_id: publish_resp.review_id,
            page_index: 0,
            page_size: 20,
        }).await.parse_resp().await.unwrap();
        assert_eq!(resp.data.len(), 2);
        assert!(resp.data.iter().any(|x| x.content == maintainer_comment));
        assert!(resp.data.iter().any(|x| x.content == uploader_comment));

        let uploader_comment_id = resp.data.iter()
            .find(|x| x.content == uploader_comment)
            .map(|x| x.id)
            .unwrap();

        // Delete uploader comment
        env.api.set_token(uploader.token.access_token.clone());
        let resp = env.api.post("/publish/review/comment/delete", &ReviewCommentDeleteReq {
            comment_id: uploader_comment_id,
        }).await;
        assert_is_ok(resp).await;

        env.api.set_token(maintainer.token.access_token.clone());
        let resp: ReviewCommentListResp = env.api.get_query("/publish/review/comment/list", &ReviewCommentListReq {
            review_id: publish_resp.review_id,
            page_index: 0,
            page_size: 20,
        }).await.parse_resp().await.unwrap();
        assert_eq!(resp.data.len(), 1);
        assert_eq!(resp.data[0].content, maintainer_comment);
    }).await;
}

#[tokio::test]
async fn test_review_comment_should_fail_with_random_user() {
    with_test_environment(|mut env| async move {
        let _uploader = with_new_random_test_user(&mut env).await;
        let publish_resp: PublishResp = env.api.post("/publish/publish", &publish_template(&env).await)
            .await
            .parse_resp()
            .await
            .unwrap();

        // Test with random user
        let _random_user = with_new_random_test_user(&mut env).await;

        let resp = env.api.post("/publish/review/comment/create", &ReviewCommentCreateReq {
            review_id: publish_resp.review_id,
            content: "random user comment".to_string(),
        }).await.parse_resp::<()>().await;
        assert_eq!(resp.unwrap_err().code, "permission_denied");

        let resp = env.api.get_query("/publish/review/comment/list", &ReviewCommentListReq {
            review_id: publish_resp.review_id,
            page_index: 0,
            page_size: 20,
        }).await.parse_resp::<()>().await;
        assert_eq!(resp.unwrap_err().code, "permission_denied");
    }).await;
}