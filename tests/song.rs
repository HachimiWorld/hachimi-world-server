mod common;

use crate::common::auth::with_new_random_test_user;
use crate::common::{assert_is_ok, CommonParse};
use crate::common::{with_test_environment, ApiClient, TestEnvironment};
use futures::future::join_all;
use hachimi_world_server::service::song_like;
use hachimi_world_server::web::routes::song::{CreationInfo, CreationTypeInfo, DetailReq, DetailResp, ProductionItem, PublishReq, PublishResp, SearchReq, SearchResp, SongListResp, TagCreateReq, TagSearchReq, TagSearchResp, UploadAudioFileResp, UploadImageResp};
use reqwest::multipart::{Form, Part};
use std::fs;
use std::time::SystemTime;


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

        // Test detail
        let resp: DetailResp = env.api.get_query("/song/detail", &DetailReq { id: last_song_display_id.clone() })
            .await.parse_resp().await.unwrap();
        assert_eq!(test_song_titles.last().unwrap().to_string(), resp.title);


        // Test search
        let search_result: SearchResp = env.api.get_query("/song/search", &SearchReq {
            q: "基米".to_string(),
            limit: None,
            offset: None,
            filter: None,
        }).await.parse_resp().await.unwrap();
        println!("{:#?}", search_result);

        let first = search_result.hits.first().unwrap();
        let first_song_id = first.id;
        click_farming_likes(&env, first_song_id, 60000).await;

        // Get likes
        let resp: DetailResp = env.api.get_query("/song/detail", &DetailReq { id: first.display_id.clone() }).await.parse_resp().await.unwrap();
        println!("{:#?}", resp);
    })
    .await;
}

#[tokio::test]
async fn test_get_likes() {
    with_test_environment(|mut env| async move {
        // Get likes
        let resp: DetailResp = env.api.get_query("/song/detail", &DetailReq { id: "JM-IOEW-474".to_string() }).await.parse_resp().await.unwrap();
        println!("{:#?}", resp);
    }).await;
}

#[tokio::test]
async fn test_discover_songs() {
    with_test_environment(|mut env| async move {
        let resp: SongListResp = env.api.get("/song/hot").await.parse_resp().await.unwrap();
        println!("hot: {:?}", resp.song_ids);

        let resp: SongListResp = env.api.get("/song/recent").await.parse_resp().await.unwrap();
        println!("recent: {:?}", resp.song_ids);
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

async fn click_farming_likes(env: &TestEnvironment, song_id: i64, number: i64) {
    // Test bench for likes
    let start = SystemTime::now();
    let mut handles = vec![];

    for _ in 0..number {
        let handle = tokio::spawn({
            let conn = env.redis.clone();
            let pool = env.pool.clone();
            async move {
                song_like::like(&conn, &pool, song_id, rand::random()).await.unwrap();
            }
        });
        handles.push(handle);
    }
    join_all(handles).await;
    println!("Spend {:.2} secs to execute 60000 likes", start.elapsed().unwrap().as_secs_f64());

}