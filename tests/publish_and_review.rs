mod common;

use crate::common::auth::{with_new_random_test_user, with_new_test_user, with_test_contributor_user};
use crate::common::{assert_is_err, CommonParse, TestEnvironment};
use crate::common::{assert_is_ok, with_test_environment, ApiClient};
use chrono::Utc;
use hachimi_world_server::db::creator::{Creator, CreatorDao};
use hachimi_world_server::db::CrudDao;
use hachimi_world_server::service::song::{CreationTypeInfo, ExternalLink};
use hachimi_world_server::web::routes::publish::{ApproveReviewReq, CreationInfo, JmidCheckPReq, JmidCheckPResp, JmidMeResp, PageReq, PageResp, ProductionItem, PublishReq, PublishResp, RejectReviewReq, UploadAudioFileResp, UploadImageResp};
use hachimi_world_server::web::routes::song::{DetailReq, DetailResp, TagCreateReq, TagSearchReq, TagSearchResp};
use reqwest::multipart::{Form, Part};
use std::fs;
use std::time::Duration;
use tokio::time;

#[tokio::test]
async fn test_publish_with_random_jmid() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        // Create tags
        // create_tags(&env.api).await;

        // Get tag
        let resp: TagSearchResp = env.api.get_query(
            "/song/tag/search",
            &TagSearchReq { query: "原教".to_string() },
        ).await.parse_resp().await.unwrap();

        let first_tag = resp.result.first().unwrap();
        assert_eq!("原教旨", first_tag.name);
        assert_eq!(None, first_tag.description);

        // Upload a song
        let upload_resp: UploadAudioFileResp = env.api
            .post_raw("/song/upload_audio_file")
            .multipart(Form::new().part("file", Part::bytes(fs::read(".local/test.mp3").unwrap())))
            .send().await.unwrap().parse_resp().await.unwrap();

        // Upload a cover
        let upload_img_resp: UploadImageResp = env
            .api
            .post_raw("/song/upload_cover_image")
            .multipart(Form::new().part("file", Part::bytes(fs::read(".local/test.webp").unwrap())))
            .send().await.unwrap().parse_resp().await.unwrap();

        // Publish a song
        let test_song_titles = vec!["不再曼波", "跳楼基"];

        let mut last_song_display_id = String::new();

        for title in &test_song_titles {
            let resp: PublishResp = env
                .api
                .post(
                    "/song/publish",
                    &PublishReq {
                        song_temp_id: upload_resp.temp_id.clone(),
                        cover_temp_id: upload_img_resp.temp_id.clone(),
                        title: title.to_string(),
                        subtitle: "A test music".to_string(),
                        description: "This is a fucking test music".to_string(),
                        lyrics: "哈基米哈基米哈基米".to_string(),
                        tag_ids: vec![first_tag.id],
                        creation_info: CreationInfo {
                            creation_type: 0,
                            origin_info: Some(CreationTypeInfo {
                                song_display_id: None,
                                title: Some("原作".into()),
                                artist: Some("群星".into()),
                                url: None,
                                origin_type: 0,
                            }),
                            derivative_info: None,
                        },
                        production_crew: vec![
                            ProductionItem {
                                role: "混音".to_string(),
                                uid: None,
                                name: Some("张三".to_string()),
                            },
                            ProductionItem {
                                role: "编曲".to_string(),
                                uid: Some(user.uid),
                                name: None,
                            },
                        ],
                        external_links: vec![
                            ExternalLink {
                                platform: "bilibili".to_string(),
                                url: "https://www.bilibili.com/video/av114514/".to_string(),
                            }
                        ],
                        explicit: Some(false),
                        jmid: None,
                        comment: None,
                    },
                )
                .await.parse_resp().await.unwrap();

            last_song_display_id = resp.song_display_id;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        let resp: PageResp = env.api.get_query("/publish/review/page", &PageReq {
            page_index: 0,
            page_size: 20,
        }).await.parse_resp().await.unwrap();
        assert_eq!(resp.data.len(), test_song_titles.len());

        let contributor_user = with_test_contributor_user(&mut env).await;

        // Test get the submitted review
        let resp: PageResp = env.api.get_query("/publish/review/page_contributor", &PageReq {
            page_index: 0,
            page_size: 20,
        }).await.parse_resp().await.unwrap();
        let first_review = resp.data.first().unwrap();
        let second_review = resp.data.get(1).unwrap();
        assert_eq!(first_review.display_id, last_song_display_id);

        // Test reject second review
        let resp = env.api.post("/publish/review/reject", &RejectReviewReq {
            review_id: second_review.review_id,
            comment: "Reject for testing".to_string(),
        }).await;
        assert_is_ok(resp).await;
        let resp = env.api.get_query("/song/detail", &DetailReq { id: second_review.display_id.clone() })
            .await.parse_resp::<DetailResp>().await;
        assert!(resp.is_err());

        // Test approve first review
        let resp = env.api.post("/publish/review/approve", &ApproveReviewReq {
            review_id: first_review.review_id,
            comment: Some("Approve for testing".to_string()),
        }).await;
        assert_is_ok(resp).await;

        // Test detail
        let resp: DetailResp = env.api.get_query("/song/detail", &DetailReq { id: last_song_display_id.clone() })
            .await.parse_resp().await.unwrap();
        assert_eq!(test_song_titles.last().unwrap().to_string(), resp.title);
    }).await;
}

async fn create_tags(api: &ApiClient) {
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

#[tokio::test]
async fn test_get_reviews() {
    with_test_environment(|mut env| async move {
        let _contributor_user = with_test_contributor_user(&mut env).await;
        let resp: PageResp = env.api.get_query("/publish/review/page_contributor", &PageReq {
            page_index: 0,
            page_size: 20,
        }).await.parse_resp().await.unwrap();
        println!("{:?}", resp);
    }).await
}

#[tokio::test]
async fn test_grant() {
    with_test_environment(|mut env| async move {
        let contributor_user = with_new_test_user(&mut env, "contributor1@example.com").await;
        // env.api.post("/review/grant", &GrantReq {})
    }).await
}

#[tokio::test]
async fn test_check_jmid() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        let resp = env.api.get("/publish/jmid/me")
            .await.parse_resp::<JmidMeResp>().await.unwrap();
        assert_eq!(resp.jmid, None);

        let resp = env.api.get("/publish/jmid/get_next")
            .await;
        assert_is_err(resp).await;

        // This code is never used
        let resp = env.api.get_query("/publish/jmid/check_prefix", &JmidCheckPReq {
            jmid: "ZJDB".to_string(),
        }).await.parse_resp::<JmidCheckPResp>().await.unwrap();
        assert_eq!(resp.result, true);

        // This code is in the song_publishing_review old data, should be available
        let resp = env.api.get_query("/publish/jmid/check_prefix", &JmidCheckPReq {
            jmid: "YCGU".to_string(),
        }).await.parse_resp::<JmidCheckPResp>().await.unwrap();
        assert_eq!(resp.result, true);

        // This code is used, should not available
        CreatorDao::insert(&env.pool, &Creator {
            id: 0,
            user_id: 0,
            jmid_prefix: "".to_string(),
            active: false,
            create_time: Utc::now(),
            update_time: Utc::now(),
        }).await.unwrap();

        let resp = env.api.get_query("/publish/jmid/check_prefix", &JmidCheckPReq {
            jmid: "".to_string(),
        }).await.parse_resp::<JmidCheckPResp>().await.unwrap();
        assert_eq!(resp.result, false);
    }).await
}

#[tokio::test]
async fn test_publish_with_jmid() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        // Publish first song, this jmid is `never_used`
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-001".into());
        let publish_001_resp: PublishResp = env.api.post("/song/publish", &req)
            .await.parse_resp().await.unwrap();

        time::sleep(Duration::from_secs(1)).await;

        // Test the `locked_by_self` logic, should fail
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-002".into());
        let resp = env.api.post("/song/publish", &req)
            .await.parse_resp::<PublishResp>().await;
        assert_eq!(resp.unwrap_err().code, "pending");

        time::sleep(Duration::from_secs(1)).await;

        // Create a new user and test the `used(locked)` logic, should fail
        let user2 = with_new_random_test_user(&mut env).await;
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-003".into());
        let resp = env.api.post("/song/publish", &req)
            .await.parse_resp::<PublishResp>().await;
        assert_eq!(resp.unwrap_err().code, "jmid_prefix_already_used");

        time::sleep(Duration::from_secs(1)).await;

        let contributor_user = with_test_contributor_user(&mut env).await;

        // Reject ABCD-001, thus release the prefix "ABCD"
        let resp = env.api.post("/publish/review/reject", &RejectReviewReq {
            review_id: publish_001_resp.review_id,
            comment: "Reject for testing".into(),
        }).await;
        assert_is_ok(resp).await;

        time::sleep(Duration::from_secs(1)).await;

        // Let user2 publish with prefix ABCD because it has been released
        env.api.set_token(user2.token.access_token.clone());
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-003".into());
        let publish_001_resp2 = env.api.post("/song/publish", &req).await
            .parse_resp::<PublishResp>().await
            .unwrap();

        time::sleep(Duration::from_secs(1)).await;

        // Approve it
        env.api.set_token(contributor_user.token.access_token);
        let resp = env.api.post("/publish/review/approve", &ApproveReviewReq {
            review_id: publish_001_resp2.review_id,
            comment: Some("Approve for testing".to_string()),
        }).await;
        assert_is_ok(resp).await;

        time::sleep(Duration::from_secs(1)).await;

        // The user2 has owned the prefix "ABCD", so he can publish again
        env.api.set_token(user2.token.access_token);
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-002".into());
        let resp = env.api.post("/song/publish", &req)
            .await.parse_resp::<PublishResp>().await;
        assert!(resp.is_ok());

        // Publish again, the jmid ABCD-002 is already im use, should fail
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-002".into());
        let resp = env.api.post("/song/publish", &req)
            .await.parse_resp::<PublishResp>().await;
        assert_eq!(resp.unwrap_err().code, "jmid_already_used");

        // Publish with another prefix, should fail
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-EFGH-001".into());
        let resp = env.api.post("/song/publish", &req)
            .await.parse_resp::<PublishResp>().await;
        assert_eq!(resp.unwrap_err().code, "jmid_prefix_mismatch");

        // Create a new user and test the `used(owned by another user)` logic
        env.api.set_token(user.token.access_token);
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-003".into());
        let resp = env.api.post("/song/publish", &req)
            .await.parse_resp::<PublishResp>().await;
        assert_eq!(resp.unwrap_err().code, "jmid_prefix_already_used");
    }).await
}

async fn publish_template(env: &TestEnvironment) -> PublishReq {
    // Upload a song
    let upload_resp: UploadAudioFileResp = env.api
        .post_raw("/song/upload_audio_file")
        .multipart(Form::new().part("file", Part::bytes(fs::read(".local/test.mp3").unwrap())))
        .send().await.unwrap().parse_resp().await.unwrap();

    // Upload a cover
    let upload_img_resp: UploadImageResp = env
        .api
        .post_raw("/song/upload_cover_image")
        .multipart(Form::new().part("file", Part::bytes(fs::read(".local/test.webp").unwrap())))
        .send().await.unwrap().parse_resp().await.unwrap();

    PublishReq {
        song_temp_id: upload_resp.temp_id.clone(),
        cover_temp_id: upload_img_resp.temp_id.clone(),
        title: "Test".to_string(),
        subtitle: "A test music".to_string(),
        description: "This is a fucking test music".to_string(),
        lyrics: "哈基米哈基米哈基米".to_string(),
        tag_ids: vec![],
        creation_info: CreationInfo {
            creation_type: 0,
            origin_info: Some(CreationTypeInfo {
                song_display_id: None,
                title: Some("原作".into()),
                artist: Some("群星".into()),
                url: None,
                origin_type: 0,
            }),
            derivative_info: None,
        },
        production_crew: vec![],
        external_links: vec![],
        explicit: Some(false),
        jmid: Some("JM-ABCD-000".into()),
        comment: Some("Test comment in review".into()),
    }
}