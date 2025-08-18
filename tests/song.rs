use crate::common::auth::with_new_random_test_user;
use crate::common::{ApiClient, with_test_environment};
use crate::common::{CommonParse, assert_is_ok};
use hachimi_world_server::web::routes::song::{
    CreationInfo, CreationTypeInfo, ProductionItem, PublishReq, PublishResp, TagCreateReq,
    TagSearchReq, TagSearchResp, UploadAudioFileResp, UploadImageResp,
};
use reqwest::multipart::{Form, Part};
use std::fs;

mod common;

#[tokio::test]
async fn test_publish() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        // Create tags
        create_tags(&env.api).await;

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

        let resp: PublishResp = env
            .api
            .post(
                "/song/publish",
                &PublishReq {
                    song_temp_id: upload_resp.temp_id,
                    cover_temp_id: upload_img_resp.temp_id,
                    title: "Test Music".to_string(),
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
        println!("{}", resp.song_display_id);
    })
    .await;
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
