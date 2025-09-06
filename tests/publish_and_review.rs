mod common;

use crate::common::CommonParse;
use crate::common::auth::{with_new_random_test_user, with_new_test_user, with_test_contributor_user};
use crate::common::{assert_is_ok, with_test_environment, ApiClient};
use hachimi_world_server::web::routes::song::{CreationInfo, CreationTypeInfo, DetailReq, DetailResp, ProductionItem, PublishReq, PublishResp, SearchReq, SearchResp, TagCreateReq, TagSearchReq, TagSearchResp, UploadAudioFileResp, UploadImageResp};
use reqwest::multipart::{Form, Part};
use std::fs;
use hachimi_world_server::web::routes::publish::{ApproveReviewReq, PageReq, PageResp, RejectReviewReq};

#[tokio::test]
async fn test_publish() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        // Create tags
        // create_tags(&env.api).await;

        // Get tag
        let resp: TagSearchResp = env
            .api
            .get_query(
                "/song/tag/search",
                &TagSearchReq {
                    query: "原教".to_string(),
                },
            )
            .await
            .parse_resp()
            .await
            .unwrap();

        let first_tag = resp.result.first().unwrap();
        assert_eq!("原教旨", first_tag.name);
        assert_eq!(None, first_tag.description);

        // Upload a song
        let upload_resp: UploadAudioFileResp = env
            .api
            .post_raw("/song/upload_audio_file")
            .multipart(Form::new().part("file", Part::bytes(fs::read(".local/test.mp3").unwrap())))
            .send()
            .await
            .unwrap()
            .parse_resp()
            .await
            .unwrap();

        // Upload a cover
        let upload_img_resp: UploadImageResp = env
            .api
            .post_raw("/song/upload_cover_image")
            .multipart(Form::new().part("file", Part::bytes(fs::read(".local/test.webp").unwrap())))
            .send()
            .await
            .unwrap()
            .parse_resp()
            .await
            .unwrap();

        // Publish a song
        let test_song_titles = vec!["不再曼波", "跳楼基", "但愿人长久", "我无怨无悔", "讨厌哈基米", "基米大厅演奏卡门序曲", "野哈飞舞", "西班牙斗耄士进行曲", "哈基博士", "为你哈基", "哈气之风", "基米没茅台", "太空曼波", "哈气的咪被火葬", "哈基米是世界的意思", "她站在地球的另一边看哈气", "兰哈草", "孤独的哈基米", "夜曲"];

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
                        external_links: vec![],
                    },
                )
                .await
                .parse_resp()
                .await
                .unwrap();

            last_song_display_id = resp.song_display_id;
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
            review_id: first_review.review_id, comment: Some("Approve for testing".to_string())
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
        let contributor_user = with_test_contributor_user(&mut env).await;
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