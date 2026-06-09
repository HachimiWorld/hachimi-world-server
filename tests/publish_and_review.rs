mod common;

use crate::common::auth::{with_new_random_test_user, with_test_contributor_user};
use crate::common::publish::{publish_template, read_test_mp3};
use crate::common::res_utils::generate_test_image;
use crate::common::{assert_is_err, ApiResult, CommonParse};
use crate::common::{assert_is_ok, with_test_environment, ApiClient};
use hachimi_world_server::service::song::{CreationTypeInfo, ExternalLink};
use hachimi_world_server::web::routes::publish::review::{ApproveReviewReq, RejectReviewReq, ReviewHistoryListReq, ReviewHistoryListResp, ReviewModifyReq};
use hachimi_world_server::web::routes::publish::{review, CreationInfo, PageReq, PageResp, ProductionItem, PublishReq, PublishResp, UploadAudioFileResp, UploadImageResp};
use hachimi_world_server::web::routes::song::{DetailReq, DetailResp, TagCreateReq, TagCreateResp, TagItem, TagSearchReq, TagSearchResp};
use image::ImageFormat;
use itertools::Itertools;
use reqwest::multipart::{Form, Part};
use reqwest::StatusCode;
use std::time::Duration;
use tokio::time;

#[tokio::test]
async fn test_create_tag_then_search() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;
        let resp = env.api.post(
            "/song/tag/create",
            &TagCreateReq {
                name: "Test".to_string(),
                description: None,
            },
        ).await;
        assert_is_ok(resp).await;

        let resp: TagSearchResp = env.api.get_query("/song/tag/search", &TagSearchReq {
            query: "Test".to_string(),
        }).await.parse_resp().await.unwrap();

        assert_eq!("Test", resp.result.first().unwrap().name);
    }).await
}

#[tokio::test]
async fn test_create_tag_should_error_when_duplicated() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        let resp = env.api.post(
            "/song/tag/create",
            &TagCreateReq {
                name: "Test".to_string(),
                description: None,
            },
        ).await;
        assert_is_ok(resp).await;

        let resp = env.api.post(
            "/song/tag/create",
            &TagCreateReq {
                name: "Test".to_string(),
                description: None,
            },
        ).await;
        assert_is_err(resp).await;
    }).await
}

#[tokio::test]
async fn test_upload_audio() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        let test_mp3_bytes = read_test_mp3();
        let resp: UploadAudioFileResp = env.api
            .post_raw("/publish/upload_audio_file")
            .multipart(Form::new().part("file", Part::bytes(test_mp3_bytes)))
            .send().await.unwrap()
            .parse_resp().await.unwrap();

        assert!(!resp.temp_id.is_empty(), "temp_id should not be empty");
        assert_eq!(10, resp.duration_secs);
        assert_eq!(Some("Test Track".into()), resp.title);
    }).await
}

#[tokio::test]
async fn test_upload_audio_should_fail_when_not_login() {
    with_test_environment(|env| async move {
        let test_mp3_bytes = read_test_mp3();
        let resp = env.api
            .post_raw("/publish/upload_audio_file")
            .multipart(Form::new().part("file", Part::bytes(test_mp3_bytes)))
            .send().await.unwrap();
        assert_eq!(StatusCode::UNAUTHORIZED, resp.status())
    }).await
}

#[tokio::test]
async fn test_upload_cover_image() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        let img_bytes = generate_test_image(128, 128, ImageFormat::Png);
        let resp: UploadImageResp = env.api
            .post_raw("/publish/upload_cover_image")
            .multipart(Form::new().part("file", Part::bytes(img_bytes)))
            .send().await.unwrap()
            .parse_resp().await.unwrap();

        assert!(!resp.temp_id.is_empty(), "temp_id should not be empty");
    }).await
}

#[tokio::test]
async fn test_publish_song_with_minium_data() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        // Upload a song
        let test_mp3_bytes = read_test_mp3();
        let upload_resp: UploadAudioFileResp = env.api
            .post_raw("/publish/upload_audio_file")
            .multipart(Form::new().part("file", Part::bytes(test_mp3_bytes)))
            .send().await.unwrap().parse_resp().await.unwrap();

        // Upload a cover
        let upload_img_resp: UploadImageResp = env.api
            .post_raw("/publish/upload_cover_image")
            .multipart(Form::new().part("file", Part::bytes(generate_test_image(128, 128, ImageFormat::Png))))
            .send().await.unwrap().parse_resp().await.unwrap();

        // Publish a song without cover and tags
        let resp = env.api.post(
            "/publish/publish",
            &PublishReq {
                song_temp_id: upload_resp.temp_id.clone(),
                cover_temp_id: upload_img_resp.temp_id.clone(),
                title: "test".to_string(),
                subtitle: "".to_string(),
                description: "".to_string(),
                lyrics: "".to_string(),
                tag_ids: vec![],
                creation_info: CreationInfo {
                    creation_type: 0,
                    origin_info: None,
                    derivative_info: None,
                },
                production_crew: vec![],
                external_links: vec![],
                explicit: Some(false),
                jmid: Some("JM-TEST-001".into()),
                comment: None,
            },
        ).await.parse_resp::<PublishResp>().await;
        assert!(resp.is_ok());
    }).await
}

#[tokio::test]
async fn test_publish_should_compatible_when_jmid_not_specified() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        // Create tags
        let tags = create_tags(&env.api).await;
        let song = publish_test_song(&env.api, None, tags.first().unwrap().id, "Test Song", None).await.unwrap();
        assert!(!song.song_display_id.is_empty(), "song_display_id should not be empty");
    }).await;
}

async fn create_tags(api: &ApiClient) -> Vec<TagItem> {
    let mut results = vec![];
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
            .await
            .parse_resp::<TagCreateResp>().await.unwrap();
        results.push(TagItem {
            id: resp.id,
            name: x.to_string(),
            description: None,
        });
    }
    results
}

async fn publish_test_song(
    api: &ApiClient,
    jmid: Option<String>,
    tag_id: i64,
    title: &str,
    additional_author_uid: Option<i64>,
) -> ApiResult<PublishResp> {
    // Upload a song
    let test_mp3_bytes = read_test_mp3();
    let upload_resp: UploadAudioFileResp = api
        .post_raw("/publish/upload_audio_file")
        .multipart(Form::new().part("file", Part::bytes(test_mp3_bytes)))
        .send().await.unwrap().parse_resp().await.unwrap();

    // Upload a cover
    let upload_img_resp: UploadImageResp = api
        .post_raw("/publish/upload_cover_image")
        .multipart(Form::new().part("file", Part::bytes(generate_test_image(128, 128, ImageFormat::Png))))
        .send().await.unwrap().parse_resp().await.unwrap();

    // Publish a song
    let resp = api.post(
        "/publish/publish",
        &PublishReq {
            song_temp_id: upload_resp.temp_id.clone(),
            cover_temp_id: upload_img_resp.temp_id.clone(),
            title: title.to_string(),
            subtitle: "Test subtitle".to_string(),
            description: "This is a test description".to_string(),
            lyrics: "哈基米哈基米哈基米".to_string(),
            tag_ids: vec![tag_id],
            creation_info: CreationInfo {
                creation_type: 1,
                origin_info: Some(CreationTypeInfo {
                    song_display_id: None,
                    title: Some("原作".into()),
                    artist: Some("群星".into()),
                    url: None,
                    origin_type: 0,
                }),
                derivative_info: None,
            },
            production_crew: match additional_author_uid {
                Some(uid) => vec![
                    ProductionItem {
                        role: "混音".to_string(),
                        uid: None,
                        name: Some("张三".to_string()),
                    },
                    ProductionItem {
                        role: "合作".to_string(),
                        uid: Some(uid),
                        name: Some("李四".to_string()), // Should be overridden by the user name in response, just for testing
                    },
                ],
                None => vec![
                    ProductionItem {
                        role: "混音".to_string(),
                        uid: None,
                        name: Some("张三".to_string()),
                    }
                ]
            },
            external_links: vec![
                ExternalLink {
                    platform: "bilibili".to_string(),
                    url: "https://www.bilibili.com/video/av114514/".to_string(),
                }
            ],
            explicit: Some(false),
            jmid,
            comment: None,
        })
        .await.parse_resp::<PublishResp>().await;
    resp
}

#[tokio::test]
async fn test_get_reviews() {
    with_test_environment(|mut env| async move {
        let _contributor_user = with_test_contributor_user(&mut env).await;
        let resp: PageResp = env.api.get_query("/publish/review/page_contributor", &PageReq {
            page_index: 0,
            page_size: 20,
        }).await.parse_resp().await.unwrap();

        // Fresh DB should have 0 reviews for contributor
        assert_eq!(resp.page_index, 0);
        assert_eq!(resp.page_size, 20);
        assert_eq!(resp.total, 0);
        assert!(resp.data.is_empty());
    }).await
}

#[tokio::test]
async fn test_approve_publishing_then_verify_public_detail() {
    with_test_environment(|mut env| async move {
        let user_additional_author = with_new_random_test_user(&mut env).await;
        let user = with_new_random_test_user(&mut env).await;
        let tags = create_tags(&env.api).await;
        let tag = tags.first().unwrap();

        let publish_resp = publish_test_song(&env.api, Some("JM-TEST-001".into()), tag.id, "Test Song For Grant", Some(user_additional_author.uid)).await.unwrap();

        // Switch to contributor and approve
        let _contributor_user = with_test_contributor_user(&mut env).await;
        let resp = env.api.post(
            "/publish/review/approve",
            &ApproveReviewReq {
                review_id: publish_resp.review_id,
                comment: Some("Approve for testing".to_string()),
            },
        ).await;

        assert_is_ok(resp).await;

        // Switch to guest user then verify the song detail
        env.api.clear_token();
        let detail = env.api.get_query("/song/detail", &DetailReq {
            id: publish_resp.song_display_id.clone(),
        }).await.parse_resp::<DetailResp>().await.unwrap();

        assert_eq!("JM-TEST-001", detail.display_id);
        assert_eq!("Test Song For Grant", detail.title);
        assert_eq!("Test subtitle", detail.subtitle);
        assert_eq!("This is a test description", detail.description);
        assert_eq!(10, detail.duration_seconds);
        assert_eq!("哈基米哈基米哈基米", detail.lyrics);

        assert!(!detail.audio_url.is_empty(), "audio_url should not be empty");
        assert!(!detail.cover_url.is_empty(), "cover_url should not be empty");

        assert_eq!(user.uid, detail.uploader_uid);
        assert_eq!(user.name, detail.uploader_name);
        assert_ne!(Some(0f32), detail.gain);
        assert_eq!(Some(false), detail.explicit);

        // Check tags (data from create_tags)
        assert_eq!(1, detail.tags.len());
        let resp_tag = detail.tags.first().unwrap();
        assert_eq!(tag.id, resp_tag.id);
        assert_eq!(tag.name, resp_tag.name);
        assert_eq!(tag.description, resp_tag.description);

        // Check production crew
        assert_eq!(2, detail.production_crew.len());
        let sorted_by_uid = detail.production_crew.iter().sorted_by(|a, b| a.uid.cmp(&b.uid)).collect_vec();
        let crew = sorted_by_uid.first().unwrap();
        assert_eq!("混音", crew.role);
        assert_eq!(Some("张三".to_string()), crew.person_name);
        assert_eq!(None, crew.uid);

        let additional = sorted_by_uid.last().unwrap();
        assert_eq!("合作", additional.role);
        assert_eq!(Some(user_additional_author.name), additional.person_name);
        assert_eq!(Some(user_additional_author.uid), additional.uid);

        // Check origin infos
        assert_eq!(1, detail.creation_type);
        assert_eq!(1, detail.origin_infos.len());
        let origin_info = detail.origin_infos.first().unwrap();
        assert_eq!(Some("原作".into()), origin_info.title);
        assert_eq!(Some("群星".into()), origin_info.artist);
        assert_eq!(0, origin_info.origin_type);
        assert_eq!(None, origin_info.song_display_id);
        assert_eq!(None, origin_info.url);

        // Check external links
        assert_eq!(1, detail.external_links.len());
        let external_link = detail.external_links.first().unwrap();
        assert_eq!("bilibili", external_link.platform);
        assert_eq!("https://www.bilibili.com/video/av114514/", external_link.url);
    }).await
}

#[tokio::test]
async fn test_publish_should_fail_when_first_publication_is_not_finished_yet() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        // Publish first song should succeed when JMID is never used before.
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-001".into());
        let resp = env.api.post("/publish/publish", &req).await;
        assert_is_ok(resp).await;

        // Wait for redis lock to be unlocked asynchronously
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Publish should fail when first publication is not finished yet.
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-002".into());
        let resp = env.api.post("/publish/publish", &req)
            .await.parse_resp::<PublishResp>().await;
        assert_eq!(resp.unwrap_err().code, "pending");

        // Wait for redis lock to be unlocked asynchronously
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Even another jmid prefix
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-BCDE-001".into());
        let resp = env.api.post("/publish/publish", &req)
            .await.parse_resp::<PublishResp>().await;
        assert_eq!(resp.unwrap_err().code, "pending");
    }).await
}

#[tokio::test]
async fn test_publish_should_fail_when_jmid_prefix_hold_by_others() {
    with_test_environment(|mut env| async move {
        let _user1 = with_new_random_test_user(&mut env).await;

        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-001".into());
        let publish_001_resp: PublishResp = env.api.post("/publish/publish", &req)
            .await.parse_resp().await.unwrap();

        time::sleep(Duration::from_millis(100)).await;

        // User 2 should fail to publish with prefix "ABCD" because it's hold by user 1
        let user2 = with_new_random_test_user(&mut env).await;
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-999".into());
        let resp = env.api.post("/publish/publish", &req)
            .await.parse_resp::<PublishResp>().await;
        assert_eq!(resp.unwrap_err().code, "jmid_prefix_already_used");

        time::sleep(Duration::from_millis(100)).await;

        // Reject the review, thus release the prefix "ABCD"
        let _contributor = with_test_contributor_user(&mut env).await;
        let resp = env.api.post("/publish/review/reject", &RejectReviewReq {
            review_id: publish_001_resp.review_id,
            comment: "Reject for testing".into(),
        }).await;
        assert_is_ok(resp).await;

        time::sleep(Duration::from_millis(100)).await;

        // User 2 should be able to publish with prefix "ABCD" because it's released after rejection
        env.api.set_token(user2.token.access_token.clone());
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-002".into());
        let publish_001_resp2 = env.api.post("/publish/publish", &req).await
            .parse_resp::<PublishResp>().await;
        assert!(publish_001_resp2.is_ok())
    }).await
}

#[tokio::test]
async fn test_publish_with_another_prefix_should_fail() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        // 1. Publish with ABCD prefix
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-ABCD-001".into());
        let abcd_resp: PublishResp = env.api.post("/publish/publish", &req)
            .await.parse_resp().await.unwrap();

        time::sleep(Duration::from_millis(100)).await;

        // 2. Let contributor approve it
        let contributor_user = with_test_contributor_user(&mut env).await;
        env.api.set_token(contributor_user.token.access_token);
        let resp = env.api.post("/publish/review/approve", &ApproveReviewReq {
            review_id: abcd_resp.review_id,
            comment: Some("Approve for testing".to_string()),
        }).await;
        assert_is_ok(resp).await;

        time::sleep(Duration::from_millis(100)).await;

        // 3. Publish with another prefix, should fail (jmid_prefix_mismatch)
        env.api.set_token(user.token.access_token);
        let mut req = publish_template(&env).await;
        req.jmid = Some("JM-EFGH-001".into());
        let resp = env.api.post("/publish/publish", &req)
            .await.parse_resp::<PublishResp>().await;
        assert_eq!(resp.unwrap_err().code, "jmid_prefix_mismatch");
    }).await
}


#[tokio::test]
async fn test_modify_review_then_get_history() {
    with_test_environment(|mut env| async move {
        let uploader = with_new_random_test_user(&mut env).await;
        let template = publish_template(&env).await;
        let publish_resp: PublishResp = env.api.post("/publish/publish", &template)
            .await
            .parse_resp()
            .await
            .unwrap();
        time::sleep(Duration::from_millis(100)).await;

        let updated_title = "Updated Test Title".to_string();
        let updated_subtitle = "Updated subtitle".to_string();
        let updated_description = "Updated description".to_string();
        let updated_lyrics = "Updated lyrics".to_string();
        let updated_comment = Some("Updated note for contributors".to_string());

        let resp = env.api.post("/publish/review/modify", &ReviewModifyReq {
            review_id: publish_resp.review_id,
            song_temp_id: None,
            cover_temp_id: None,
            title: updated_title.clone(),
            subtitle: updated_subtitle.clone(),
            description: updated_description.clone(),
            lyrics: updated_lyrics.clone(),
            tag_ids: template.tag_ids.clone(),
            creation_info: template.creation_info.clone(),
            production_crew: template.production_crew.clone(),
            external_links: template.external_links.clone(),
            explicit: template.explicit.unwrap_or(false),
            comment: updated_comment.clone(),
        }).await;
        assert_is_ok(resp).await;

        let detail: review::DetailResp = env.api.get_query(
            "/publish/review/detail",
            &review::DetailReq { review_id: publish_resp.review_id },
        ).await.parse_resp().await.unwrap();
        assert_eq!(detail.title, updated_title);
        assert_eq!(detail.subtitle, updated_subtitle);
        assert_eq!(detail.description, updated_description);
        assert_eq!(detail.lyrics, updated_lyrics);
        assert_eq!(detail.comment, updated_comment);

        let history: ReviewHistoryListResp = env.api.get_query(
            "/publish/review/history/list",
            &ReviewHistoryListReq {
                review_id: publish_resp.review_id,
                page_index: 0,
                page_size: 20,
            },
        ).await.parse_resp().await.unwrap();
        assert_eq!(history.total, 2);
        assert_eq!(history.data.len(), 2);
        assert_eq!(history.data[0].note, Some("Updated note for contributors".to_string()));
        assert_eq!(history.data[0].snapshot.as_ref().map(|x| x.title.clone()), Some("Updated Test Title".to_string()));

        let other_user = with_new_random_test_user(&mut env).await;
        let resp = env.api.post("/publish/review/modify", &ReviewModifyReq {
            review_id: publish_resp.review_id,
            song_temp_id: None,
            cover_temp_id: None,
            title: "Hacked".to_string(),
            subtitle: "Hacked".to_string(),
            description: "Hacked".to_string(),
            lyrics: "Hacked".to_string(),
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
            explicit: false,
            comment: Some("Should fail".to_string()),
        }).await;
        assert_is_err(resp).await;

        env.api.set_token(uploader.token.access_token.clone());
        let contributor = with_test_contributor_user(&mut env).await;
        env.api.set_token(contributor.token.access_token.clone());
        let history: ReviewHistoryListResp = env.api.get_query(
            "/publish/review/history/list",
            &ReviewHistoryListReq {
                review_id: publish_resp.review_id,
                page_index: 0,
                page_size: 20,
            },
        ).await.parse_resp().await.unwrap();
        assert_eq!(history.data.len(), 2);

        env.api.set_token(other_user.token.access_token);
        let resp = env.api.get_query(
            "/publish/review/history/list",
            &ReviewHistoryListReq {
                review_id: publish_resp.review_id,
                page_index: 0,
                page_size: 20,
            },
        ).await;
        assert_is_err(resp).await;
    }).await;
}


#[tokio::test]
async fn test_modify_review_by_other_user_should_fail() {
    with_test_environment(|mut env| async move {
        let _uploader = with_new_random_test_user(&mut env).await;
        let publish_resp: PublishResp = env.api.post("/publish/publish", &publish_template(&env).await)
            .await
            .parse_resp()
            .await
            .unwrap();

        let other_user = with_new_random_test_user(&mut env).await;
        env.api.set_token(other_user.token.access_token.clone());
        let resp = env.api.post("/publish/review/modify", &ReviewModifyReq {
            review_id: publish_resp.review_id,
            song_temp_id: None,
            cover_temp_id: None,
            title: "Hacked".to_string(),
            subtitle: "Hacked".to_string(),
            description: "Hacked".to_string(),
            lyrics: "Hacked".to_string(),
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
            explicit: false,
            comment: Some("Should fail".to_string()),
        }).await.parse_resp::<()>().await;

        assert_eq!("permission_denied", resp.unwrap_err().code);
    }).await;
}

