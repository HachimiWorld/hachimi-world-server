use crate::common::res_utils::generate_test_image;
use crate::common::{CommonParse, TestEnvironment};
use hachimi_world_server::db::song::{ISongDao, SongDao};
use hachimi_world_server::service::song::{generate_song_display_id, CreationTypeInfo};
use hachimi_world_server::web::routes::publish::jmid::JmidGetNextResp;
use hachimi_world_server::web::routes::publish::review::ApproveReviewReq;
use hachimi_world_server::web::routes::publish::{CreationInfo, PublishReq, PublishResp, UploadAudioFileResp, UploadImageResp};
use image::ImageFormat;
use reqwest::multipart::{Form, Part};
use std::fs::File;
use std::io::Read;

pub async fn publish_template(env: &TestEnvironment) -> PublishReq {
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

pub fn read_test_mp3() -> Vec<u8> {
    File::open("tests/fixtures/test-mp3.mp3").unwrap()
        .bytes()
        .map(|b| b.unwrap())
        .collect()
}

pub fn unique_jmid() -> String {
    generate_song_display_id()
}

pub struct ApprovedSong {
    pub id: i64,
    pub jmid: String,
}

/// Creates a song by publishing it as `owner_token` and approving it as `contributor_token`.
/// Returns the approved song with both internal ID and JMID.
pub async fn create_approved_song(
    env: &mut TestEnvironment,
    owner_token: &str,
    contributor_token: &str,
    title: &str,
) -> ApprovedSong {
    env.api.set_token(owner_token.to_string());

    let unique = uuid::Uuid::new_v4().simple().to_string();
    let mut req = publish_template(env).await;

    // Get next jmid or random
    let next_jmid = env.api.get("/publish/jmid/get_next").await.parse_resp::<JmidGetNextResp>().await;
    let jmid = match next_jmid {
        Ok(x) => format!("JM-{}", x.jmid),
        Err(_) => unique_jmid()
    };

    req.title = format!("{title}-{unique}");
    req.subtitle = format!("subtitle-{unique}");
    req.description = format!("description-{unique}");
    req.lyrics = format!("lyrics-{unique}");
    req.jmid = Some(jmid);

    let publish_resp = env.api.post("/publish/publish", &req).await
        .parse_resp::<PublishResp>().await.unwrap();

    env.api.set_token(contributor_token.to_string());
    env.api.post(
        "/publish/review/approve",
        &ApproveReviewReq {
            review_id: publish_resp.review_id,
            comment: Some("Approve for integration test".to_string()),
        },
    ).await.parse_resp::<()>().await.unwrap();

    env.api.set_token(owner_token.to_string());
    let id = SongDao::get_by_display_id(&env.pool, &publish_resp.song_display_id).await.unwrap().unwrap().id;
    ApprovedSong { id, jmid: publish_resp.song_display_id }
}