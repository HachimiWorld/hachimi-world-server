mod common;

use crate::common::{assert_is_ok, auth, CommonParse};
use common::with_test_environment;
use hachimi_world_server::web::routes::user::{GetProfileReq, PublicUserProfile, SearchReq, SearchResp, UpdateProfileReq};
use image::{ImageBuffer, ImageFormat, Rgb};
use reqwest::multipart::{Form, Part};

#[tokio::test]
async fn test_get_and_update_profile() {
    with_test_environment(|mut env| async move {
        let random_email = format!("test_{}@mail.com", uuid::Uuid::new_v4());

        let user = auth::with_new_test_user(&mut env, &random_email).await;

        let test_bio = "我是神人我是神人".to_string();
        let test_username = format!("我是神人{}", rand::random::<u8>());

        let resp = env.api.post("/user/update_profile", &UpdateProfileReq {
            username: test_username.clone(),
            bio: Some(test_bio.clone()),
            gender: Some(0),
        }).await;
        assert_is_ok(resp).await;

        let resp: PublicUserProfile = env.api.get_query("/user/profile", &GetProfileReq {
            uid: user.uid,
        }).await.parse_resp().await.unwrap();

        assert_eq!(Some(test_bio), resp.bio);
        assert_eq!(test_username, resp.username);
        assert_eq!(Some(0), resp.gender);
    }).await
}

#[tokio::test]
async fn test_set_avatar() {
    with_test_environment(|mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        let png = generate_test_avatar(64, 128, ImageFormat::Png);

        let resp = env.api
            .post_raw("/user/set_avatar")
            .multipart(Form::new().part("file", Part::bytes(png).file_name("avatar.png")))
            .send()
            .await
            .unwrap();
        assert_is_ok(resp).await;

        let profile: PublicUserProfile = env.api.get_query("/user/profile", &GetProfileReq {
            uid: user.uid,
        }).await.parse_resp().await.unwrap();

        let avatar_url = profile.avatar_url.expect("avatar url should be set");
        assert!(avatar_url.starts_with("https://mock-file-host/images/avatar/"), "{avatar_url}");
        assert!(avatar_url.ends_with(".webp"), "{avatar_url}");
    }).await
}

fn generate_test_avatar(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
    let mut img = ImageBuffer::new(width, height);

    for (x, y, pixel) in img.enumerate_pixels_mut() {
        *pixel = Rgb([ (x % 256) as u8, (y % 256) as u8, 128 ]);
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut bytes);
    img.write_to(&mut cursor, format).unwrap();

    bytes
}

#[tokio::test]
async fn test_search() {
    with_test_environment(|mut env| async move {
        let resp: SearchResp = env.api.get_query("/user/search", &SearchReq {
            q: "神".to_string(),
            page: 0,
            size: 20,
        }).await.parse_resp().await.unwrap();
        println!("{:?}", resp);
    }).await
}