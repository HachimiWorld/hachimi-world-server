mod common;

use common::with_test_environment;
use hachimi_world_server::web::result::WebResponse;
use hachimi_world_server::web::routes::user::{GetProfileReq, PublicUserProfile, UpdateProfileReq};
use crate::common::{assert_is_ok, auth};

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

        let resp: WebResponse<PublicUserProfile> = env.api.post("/user/profile", &GetProfileReq {
            uid: user.uid,
        }).await.json().await.unwrap();

        assert_eq!(Some(test_bio), resp.data.bio);
        assert_eq!(test_username, resp.data.username);
        assert_eq!(Some(0), resp.data.gender);
    }).await
}

#[tokio::test]
async fn test_set_avatar() {
    // TODO
}