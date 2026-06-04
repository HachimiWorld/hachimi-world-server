use crate::common::res_utils::generate_test_image;
use crate::common::{CommonParse, TestEnvironment};
use hachimi_world_server::service::song::CreationTypeInfo;
use hachimi_world_server::web::routes::publish::{CreationInfo, PublishReq, UploadAudioFileResp, UploadImageResp};
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