mod common;

use crate::common::typesafe::{get_failing_typesafe_client, get_mock_typesafe_client, new_content_checker, TEST_IMAGE_MODEL, TEST_MODEL};
use crate::common::{auth, with_test_environment, with_test_environment_with, ApiClient, CommonParse, TestOptions};
use hachimi_world_server::db::content_check::{ContentCheck, ContentCheckDao, ContentPolicyDao};
use hachimi_world_server::service::user::PublicUserProfile;
use hachimi_world_server::util::typesafe::{MockTypeSafeClient, SystemOneResponse, TypeSafeStatusError};
use hachimi_world_server::web::routes::playlist::{CreatePlaylistReq, CreatePlaylistResp, ListResp, PlaylistItem, SetCoverReq, UpdatePlaylistReq};
use hachimi_world_server::web::routes::user::{GetProfileReq, UpdateProfileReq};
use image::{ImageFormat, Rgb, RgbImage};
use reqwest::multipart::{Form, Part};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

const CHECKS_ON: TestOptions = TestOptions { content_check: true };

async fn update(api: &ApiClient, username: &str, bio: Option<&str>) -> Result<(), String> {
    api.post("/user/update_profile", &UpdateProfileReq {
        username: username.to_string(),
        bio: bio.map(str::to_string),
        gender: None,
    }).await.parse_resp::<()>().await.map_err(|e| e.code)
}

/// A PNG of one colour; the mock judges images by their colour, see [get_mock_typesafe_client].
fn png(color: [u8; 3]) -> Vec<u8> {
    let mut png = Vec::new();
    RgbImage::from_pixel(300, 200, Rgb(color)).write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png).unwrap();
    png
}

const GREY: [u8; 3] = [128, 128, 128];
/// Blocked as `sexual` by the mock
const RED: [u8; 3] = [230, 20, 20];
/// Fails the mock's call
const BLUE: [u8; 3] = [20, 20, 230];

async fn set_avatar(api: &ApiClient, png: Vec<u8>) -> Result<(), String> {
    api.post_raw("/user/set_avatar")
        .multipart(Form::new().part("file", Part::bytes(png).file_name("avatar.png")))
        .send().await.unwrap()
        .parse_resp::<()>().await.map_err(|e| e.code)
}

async fn profile(api: &ApiClient, uid: i64) -> PublicUserProfile {
    api.get_query("/user/profile", &GetProfileReq { uid }).await.parse_resp().await.unwrap()
}

/// Oldest first.
async fn checks(pool: &PgPool, uid: i64) -> Vec<ContentCheck> {
    let mut checks = ContentCheckDao::list_by_actor(pool, uid, 50).await.unwrap();
    checks.reverse();
    checks
}

#[tokio::test]
async fn ordinary_changes_pass_and_are_recorded() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        update(&env.api, "哈基米", Some("喜欢听歌")).await.unwrap();

        let profile = profile(&env.api, user.uid).await;
        assert_eq!(profile.username, "哈基米");
        assert_eq!(profile.bio.as_deref(), Some("喜欢听歌"));

        let checks = checks(&env.pool, user.uid).await;
        assert_eq!(checks.len(), 2);
        let mut subjects: Vec<_> = checks.iter().map(|c| c.subject.as_str()).collect();
        subjects.sort();
        assert_eq!(subjects, ["user.bio", "user.username"]);
        for check in &checks {
            assert_eq!(check.target_id, user.uid);
            assert!(check.blocked_by.is_empty(), "{check:?}");
            assert_eq!(check.model.as_deref(), Some(TEST_MODEL));
            assert!(check.answers.is_some());
            assert_eq!(check.error, None);
            assert_eq!(check.content_hash.len(), 64);
        }

        // Every check names a stored policy version
        let policy = ContentPolicyDao::get_by_version(&env.pool, &checks[0].policy_version).await.unwrap()
            .expect("the policy snapshot should be stored");
        assert!(policy.files.get("decision.yaml").is_some());
        assert!(policy.files.get("questions/user.username.json").is_some());
    }).await
}

#[tokio::test]
async fn a_blocked_username_is_not_saved() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;
        let before = profile(&env.api, user.uid).await;

        assert_eq!(update(&env.api, "坏名#sexual", Some("喜欢听歌")).await, Err("username_rejected".to_string()));

        // Nothing is saved, not even the bio that passed
        let after = profile(&env.api, user.uid).await;
        assert_eq!(after.username, before.username);
        assert_eq!(after.bio, before.bio);

        let checks = checks(&env.pool, user.uid).await;
        let username = checks.iter().find(|c| c.subject == "user.username").unwrap();
        assert_eq!(username.content, "坏名#sexual");
        assert_eq!(username.blocked_by, ["sexual"]);
        let bio = checks.iter().find(|c| c.subject == "user.bio").unwrap();
        assert!(bio.blocked_by.is_empty());
    }).await
}

#[tokio::test]
async fn a_blocked_bio_is_not_saved() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;
        let before = profile(&env.api, user.uid).await;

        assert_eq!(update(&env.api, "哈基米", Some("加群领福利 #spam")).await, Err("bio_rejected".to_string()));

        let after = profile(&env.api, user.uid).await;
        assert_eq!(after.username, before.username);
        assert_eq!(after.bio, before.bio);

        let bio = checks(&env.pool, user.uid).await.into_iter().find(|c| c.subject == "user.bio").unwrap();
        assert_eq!(bio.blocked_by, ["spam"]);
    }).await
}

#[tokio::test]
async fn content_passes_when_the_check_fails() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        // The call fails
        update(&env.api, "名字#down", None).await.unwrap();
        assert_eq!(profile(&env.api, user.uid).await.username, "名字#down");

        // The answers don't match the policy
        update(&env.api, "名字#odd", None).await.unwrap();
        assert_eq!(profile(&env.api, user.uid).await.username, "名字#odd");

        let checks = checks(&env.pool, user.uid).await;
        assert_eq!(checks.len(), 2);

        assert_eq!(checks[0].model, None);
        assert_eq!(checks[0].answers, None);
        assert!(checks[0].error.as_deref().unwrap().contains("503"), "{:?}", checks[0].error);
        assert!(checks[0].blocked_by.is_empty());

        assert_eq!(checks[1].model.as_deref(), Some(TEST_MODEL));
        assert!(checks[1].error.as_deref().unwrap().contains("no noul answer"), "{:?}", checks[1].error);
        assert!(checks[1].blocked_by.is_empty());
    }).await
}

#[tokio::test]
async fn only_changed_fields_are_checked() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        update(&env.api, "哈基米", Some("喜欢听歌")).await.unwrap();
        assert_eq!(checks(&env.pool, user.uid).await.len(), 2);

        // Same username, new bio
        update(&env.api, "哈基米", Some("更喜欢唱歌")).await.unwrap();
        let checks_now = checks(&env.pool, user.uid).await;
        assert_eq!(checks_now.len(), 3);
        assert_eq!(checks_now[2].subject, "user.bio");
        assert_eq!(checks_now[2].content, "更喜欢唱歌");

        // Nothing changed, and an empty bio has nothing to check
        update(&env.api, "哈基米", Some("更喜欢唱歌")).await.unwrap();
        update(&env.api, "哈基米", None).await.unwrap();
        update(&env.api, "哈基米", Some("  ")).await.unwrap();
        assert_eq!(checks(&env.pool, user.uid).await.len(), 3);
    }).await
}

#[tokio::test]
async fn nothing_is_checked_without_content_check() {
    with_test_environment(|mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        update(&env.api, "坏名#sexual", Some("加群领福利 #spam")).await.unwrap();

        assert_eq!(profile(&env.api, user.uid).await.username, "坏名#sexual");
        assert!(checks(&env.pool, user.uid).await.is_empty());
    }).await
}

#[tokio::test]
async fn text_judged_before_reuses_the_answer() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let first = auth::with_new_random_test_user(&mut env).await;
        update(&env.api, "哈基米", Some("加群领福利 #spam")).await.unwrap_err();
        let earlier = checks(&env.pool, first.uid).await.into_iter().find(|c| c.subject == "user.bio").unwrap();
        assert_eq!(earlier.reused_from, None);

        // Anyone sending the same text gets the same verdict, without asking TypeSafe
        let second = auth::with_new_random_test_user(&mut env).await;
        assert_eq!(update(&env.api, "哈基咪", Some("加群领福利 #spam")).await, Err("bio_rejected".to_string()));
        let reused = checks(&env.pool, second.uid).await.into_iter().find(|c| c.subject == "user.bio").unwrap();
        assert_eq!(reused.reused_from, Some(earlier.id));
        assert_eq!(reused.blocked_by, ["spam"]);
        assert_eq!(reused.content, "加群领福利 #spam");
        assert_eq!(reused.content_hash, earlier.content_hash);
        assert_eq!(reused.model, earlier.model);
        assert_eq!(reused.answers, earlier.answers);
        assert_eq!(reused.latency_ms, 0);

        // A reused check points at the one that asked, not at another reused one
        let third = auth::with_new_random_test_user(&mut env).await;
        update(&env.api, "哈基咩", Some("加群领福利 #spam")).await.unwrap_err();
        let again = checks(&env.pool, third.uid).await.into_iter().find(|c| c.subject == "user.bio").unwrap();
        assert_eq!(again.reused_from, Some(earlier.id));

        // Only the same subject counts: a bio that passed says nothing about a playlist description
        update(&env.api, "哈基咩", Some("喜欢听歌")).await.unwrap();
        create_playlist(&env.api, "通勤歌单", Some("喜欢听歌")).await.unwrap();
        let description = checks(&env.pool, third.uid).await.into_iter().find(|c| c.subject == "playlist.description").unwrap();
        assert_eq!(description.reused_from, None);
    }).await
}

#[tokio::test]
async fn failed_checks_and_other_policies_are_not_reused() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        // Passed only because the call failed, so it's asked again
        update(&env.api, "名字#down", None).await.unwrap();
        update(&env.api, "名字", None).await.unwrap();
        update(&env.api, "名字#down", None).await.unwrap();
        let checks_now = checks(&env.pool, user.uid).await;
        assert_eq!(checks_now.len(), 3);
        assert!(checks_now.iter().all(|c| c.reused_from.is_none()), "{checks_now:?}");

        // Judged as blocked under another policy version, or by another model
        let hash = hex::encode(Sha256::digest("喜欢听歌".as_bytes()));
        let policy_version = &checks_now[0].policy_version;
        for (version, model) in [("earlier", TEST_MODEL), (policy_version.as_str(), "jev-earlier")] {
            sqlx::query(
                "INSERT INTO content_checks (subject, target_id, actor_uid, content, content_hash, policy_version, model, answers, blocked_by, latency_ms, create_time)
                 VALUES ('user.bio', 0, 0, '喜欢听歌', $1, $2, $3, '{}', '{sexual}', 0, now())"
            ).bind(&hash).bind(version).bind(model).execute(&env.pool).await.unwrap();
        }
        update(&env.api, "名字", Some("喜欢听歌")).await.unwrap();
        let bio = checks(&env.pool, user.uid).await.into_iter().find(|c| c.subject == "user.bio").unwrap();
        assert_eq!(bio.reused_from, None);
        assert!(bio.blocked_by.is_empty());
    }).await
}

fn status(status: StatusCode) -> anyhow::Error {
    TypeSafeStatusError { status, body: "mock".to_string() }.into()
}

#[tokio::test]
async fn startup_fails_when_typesafe_rejects_the_config() {
    with_test_environment(|env| async move {
        // A bad key
        let e = new_content_checker(&env.pool, get_failing_typesafe_client(|| status(StatusCode::UNAUTHORIZED))).await
            .err().expect("a rejected key should stop startup");
        assert!(format!("{e:#}").contains("401"), "{e:#}");

        // A bad model or malformed questions
        assert!(new_content_checker(&env.pool, get_failing_typesafe_client(|| status(StatusCode::UNPROCESSABLE_ENTITY))).await.is_err());

        // Answers that don't match the policy
        let empty = |model: &str| -> SystemOneResponse {
            serde_json::from_value(serde_json::json!({ "model": model, "answers": {}, "usage": { "input_tokens": 1 } })).unwrap()
        };
        let mut mismatched = MockTypeSafeClient::default();
        mismatched.expect_system_one().returning(move |model, _, _| {
            let resp = empty(model);
            Box::pin(async move { Ok(resp) })
        });
        mismatched.expect_system_one_image().returning(move |model, _, _| {
            let resp = empty(model);
            Box::pin(async move { Ok(resp) })
        });
        let e = new_content_checker(&env.pool, mismatched).await.err().expect("mismatched answers should stop startup");
        assert!(format!("{e:#}").contains("don't match the policy"), "{e:#}");

        // Nothing was stored for a policy that didn't start
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM content_policies").fetch_one(&env.pool).await.unwrap();
        assert_eq!(count, 0);
    }).await
}

#[tokio::test]
async fn startup_goes_on_when_typesafe_is_unavailable() {
    with_test_environment(|env| async move {
        new_content_checker(&env.pool, get_failing_typesafe_client(|| anyhow::anyhow!("operation timed out"))).await.unwrap();
        new_content_checker(&env.pool, get_failing_typesafe_client(|| status(StatusCode::SERVICE_UNAVAILABLE))).await.unwrap();
        new_content_checker(&env.pool, get_failing_typesafe_client(|| status(StatusCode::TOO_MANY_REQUESTS))).await.unwrap();
        new_content_checker(&env.pool, get_mock_typesafe_client()).await.unwrap();
    }).await
}

/// The storage key in a mock file host URL.
fn key_of(url: &str) -> &str {
    url.strip_prefix("https://mock-file-host/").unwrap()
}

#[tokio::test]
async fn an_ordinary_avatar_passes_and_is_recorded() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        let uploaded = png(GREY);
        set_avatar(&env.api, uploaded.clone()).await.unwrap();
        let avatar_url = profile(&env.api, user.uid).await.avatar_url.expect("the avatar should be saved");

        let checks = checks(&env.pool, user.uid).await;
        assert_eq!(checks.len(), 1);
        let check = &checks[0];
        assert_eq!(check.subject, "user.avatar");
        assert_eq!(check.target_id, user.uid);
        assert_eq!(check.content, key_of(&avatar_url));
        // The hash is of the file as uploaded, not of the copy that was checked
        assert_eq!(check.content_hash, hex::encode(Sha256::digest(&uploaded)));
        assert_eq!(check.model.as_deref(), Some(TEST_IMAGE_MODEL));
        assert!(check.blocked_by.is_empty(), "{check:?}");
        assert_eq!(check.error, None);
    }).await
}

#[tokio::test]
async fn a_blocked_avatar_is_not_saved() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;
        set_avatar(&env.api, png(GREY)).await.unwrap();
        let before = profile(&env.api, user.uid).await.avatar_url;

        assert_eq!(set_avatar(&env.api, png(RED)).await, Err("avatar_rejected".to_string()));
        assert_eq!(profile(&env.api, user.uid).await.avatar_url, before);

        let checks = checks(&env.pool, user.uid).await;
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[1].blocked_by, ["sexual"]);
        assert!(checks[1].content.starts_with("content-check/blocked/avatar/"), "{}", checks[1].content);
        assert!(checks[1].content.ends_with(".webp"), "{}", checks[1].content);
    }).await
}

#[tokio::test]
async fn an_avatar_passes_when_the_check_fails() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        set_avatar(&env.api, png(BLUE)).await.unwrap();
        let avatar_url = profile(&env.api, user.uid).await.avatar_url.expect("the avatar should be saved");

        let checks = checks(&env.pool, user.uid).await;
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].content, key_of(&avatar_url));
        assert_eq!(checks[0].model, None);
        assert!(checks[0].error.as_deref().unwrap().contains("503"), "{:?}", checks[0].error);
        assert!(checks[0].blocked_by.is_empty());
    }).await
}

#[tokio::test]
async fn avatar_checks_are_limited_per_hour() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        // Encoded first: encoding on the test's only thread holds up Redis replies, unlocks included.
        // Each a little different, since a file judged before isn't limited
        let pngs = (0..4).map(|i| png([128, 128, 120 + i])).collect::<Vec<_>>();

        // Blocked checks count too
        set_avatar(&env.api, png(RED)).await.unwrap_err();
        // Seven more earlier this hour, so a few uploads reach the limit. Few, because they wait for
        // each other
        for _ in 0..7 {
            sqlx::query(
                "INSERT INTO content_checks (subject, target_id, actor_uid, content, content_hash, policy_version, blocked_by, latency_ms, create_time)
                 VALUES ('user.avatar', $1, $1, 'earlier', 'earlier', 'earlier', '{}', 0, now() - interval '30 minutes')"
            ).bind(user.uid).execute(&env.pool).await.unwrap();
        }

        // Uploads at the same time are counted one by one
        let results = futures::future::join_all(pngs.into_iter().map(|png| set_avatar(&env.api, png))).await;
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 2, "{results:?}");
        assert!(results.iter().filter(|r| r.is_err()).all(|r| r.as_ref().err().map(String::as_str) == Some("rate_limited")), "{results:?}");

        // A limited upload isn't checked, so it doesn't count
        assert_eq!(checks(&env.pool, user.uid).await.len(), 10);

        // Other users have their own limit, and text isn't limited
        update(&env.api, "哈基米", Some("喜欢听歌")).await.unwrap();
        let other = auth::with_new_random_test_user(&mut env).await;
        set_avatar(&env.api, png(GREY)).await.unwrap();
        assert_eq!(checks(&env.pool, other.uid).await.len(), 1);
    }).await
}

#[tokio::test]
async fn an_image_judged_before_is_blocked_at_once_and_not_limited() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let blocked = png(RED);
        let passed = png(GREY);
        let first = auth::with_new_random_test_user(&mut env).await;
        set_avatar(&env.api, blocked.clone()).await.unwrap_err();
        set_avatar(&env.api, passed.clone()).await.unwrap();
        let earlier = checks(&env.pool, first.uid).await;

        // Another user, past the limit, uploads the same files
        let second = auth::with_new_random_test_user(&mut env).await;
        for _ in 0..10 {
            sqlx::query(
                "INSERT INTO content_checks (subject, target_id, actor_uid, content, content_hash, policy_version, blocked_by, latency_ms, create_time)
                 VALUES ('user.avatar', $1, $1, 'earlier', 'earlier', 'earlier', '{}', 0, now() - interval '30 minutes')"
            ).bind(second.uid).execute(&env.pool).await.unwrap();
        }
        assert_eq!(set_avatar(&env.api, blocked).await, Err("avatar_rejected".to_string()));
        set_avatar(&env.api, passed).await.unwrap();
        let avatar_url = profile(&env.api, second.uid).await.avatar_url.expect("the avatar should be saved");
        assert_eq!(set_avatar(&env.api, png([128, 128, 120])).await, Err("rate_limited".to_string()));

        let reused: Vec<_> = checks(&env.pool, second.uid).await.into_iter().filter(|c| c.reused_from.is_some()).collect();
        assert_eq!(reused.len(), 2);
        assert_eq!(reused[0].reused_from, Some(earlier[0].id));
        assert_eq!(reused[0].blocked_by, ["sexual"]);
        assert!(reused[0].content.starts_with("content-check/blocked/avatar/"), "{}", reused[0].content);
        assert_eq!(reused[1].reused_from, Some(earlier[1].id));
        assert_eq!(reused[1].content, key_of(&avatar_url));

        // The same file as a playlist cover is judged on its own
        let id = create_playlist(&env.api, "通勤歌单", None).await.unwrap();
        assert_eq!(set_cover(&env.api, id, png(RED)).await, Err("cover_rejected".to_string()));
        let cover = checks(&env.pool, second.uid).await.into_iter().find(|c| c.subject == "playlist.cover").unwrap();
        assert_eq!(cover.reused_from, None);
    }).await
}

#[tokio::test]
async fn startup_fails_when_typesafe_rejects_the_image_model() {
    with_test_environment(|env| async move {
        let mut mock = MockTypeSafeClient::default();
        mock.expect_system_one().returning(|model, _, questions| {
            let answers: serde_json::Map<_, _> = questions.as_object().unwrap().keys()
                .map(|id| (id.clone(), serde_json::json!({ "type": "noul", "noul": 0.05 })))
                .collect();
            let resp: SystemOneResponse = serde_json::from_value(serde_json::json!({
                "model": model, "answers": answers, "usage": { "input_tokens": 1 }
            })).unwrap();
            Box::pin(async move { Ok(resp) })
        });
        mock.expect_system_one_image().returning(|_, _, _| Box::pin(async move { Err(status(StatusCode::BAD_REQUEST)) }));

        let e = new_content_checker(&env.pool, mock).await.err().expect("a rejected image model should stop startup");
        assert!(format!("{e:#}").contains(TEST_IMAGE_MODEL), "{e:#}");
    }).await
}

async fn create_playlist(api: &ApiClient, name: &str, description: Option<&str>) -> Result<i64, String> {
    api.post("/playlist/create", &CreatePlaylistReq {
        name: name.to_string(),
        description: description.map(str::to_string),
        is_public: true,
    }).await.parse_resp::<CreatePlaylistResp>().await.map(|x| x.id).map_err(|e| e.code)
}

async fn update_playlist(api: &ApiClient, id: i64, name: &str, description: Option<&str>) -> Result<(), String> {
    api.post("/playlist/update", &UpdatePlaylistReq {
        id,
        name: name.to_string(),
        description: description.map(str::to_string),
        is_public: true,
    }).await.parse_resp::<()>().await.map_err(|e| e.code)
}

async fn set_cover(api: &ApiClient, playlist_id: i64, png: Vec<u8>) -> Result<(), String> {
    let json = serde_json::to_string(&SetCoverReq { playlist_id }).unwrap();
    api.post_raw("/playlist/set_cover")
        .multipart(Form::new().text("json", json).part("file", Part::bytes(png).file_name("cover.png")))
        .send().await.unwrap()
        .parse_resp::<()>().await.map_err(|e| e.code)
}

/// The user's own playlists.
async fn playlists(api: &ApiClient) -> Vec<PlaylistItem> {
    api.get("/playlist/list").await.parse_resp::<ListResp>().await.unwrap().playlists
}

#[tokio::test]
async fn ordinary_playlists_pass_and_are_recorded() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        let id = create_playlist(&env.api, "通勤歌单", Some("上班路上听")).await.unwrap();

        let checks = checks(&env.pool, user.uid).await;
        let mut subjects: Vec<_> = checks.iter().map(|c| c.subject.as_str()).collect();
        subjects.sort();
        assert_eq!(subjects, ["playlist.description", "playlist.name"]);
        for check in &checks {
            assert_eq!(check.target_id, id);
            assert!(check.blocked_by.is_empty(), "{check:?}");
            assert_eq!(check.model.as_deref(), Some(TEST_MODEL));
        }
    }).await
}

#[tokio::test]
async fn a_blocked_playlist_is_not_created() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        assert_eq!(create_playlist(&env.api, "坏名#sexual", None).await, Err("name_rejected".to_string()));
        assert_eq!(create_playlist(&env.api, "通勤歌单", Some("加群领福利 #spam")).await, Err("description_rejected".to_string()));
        assert!(playlists(&env.api).await.is_empty());

        let checks = checks(&env.pool, user.uid).await;
        let name = checks.iter().find(|c| c.subject == "playlist.name" && !c.blocked_by.is_empty()).unwrap();
        assert_eq!(name.content, "坏名#sexual");
        assert_eq!(name.blocked_by, ["sexual"]);
        let description = checks.iter().find(|c| c.subject == "playlist.description").unwrap();
        assert_eq!(description.blocked_by, ["spam"]);
    }).await
}

#[tokio::test]
async fn a_blocked_playlist_change_is_not_saved() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        auth::with_new_random_test_user(&mut env).await;
        let id = create_playlist(&env.api, "通勤歌单", Some("上班路上听")).await.unwrap();

        assert_eq!(update_playlist(&env.api, id, "坏名#sexual", Some("上班路上听")).await, Err("name_rejected".to_string()));
        assert_eq!(update_playlist(&env.api, id, "通勤歌单", Some("加群领福利 #spam")).await, Err("description_rejected".to_string()));

        let playlist = playlists(&env.api).await.into_iter().next().unwrap();
        assert_eq!(playlist.name, "通勤歌单");
        assert_eq!(playlist.description.as_deref(), Some("上班路上听"));
    }).await
}

#[tokio::test]
async fn only_changed_playlist_fields_are_checked() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;
        let id = create_playlist(&env.api, "通勤歌单", None).await.unwrap();
        assert_eq!(checks(&env.pool, user.uid).await.len(), 1);

        // Same name, new description
        update_playlist(&env.api, id, "通勤歌单", Some("上班路上听")).await.unwrap();
        let checks_now = checks(&env.pool, user.uid).await;
        assert_eq!(checks_now.len(), 2);
        assert_eq!(checks_now[1].subject, "playlist.description");

        // Nothing changed, and an empty description has nothing to check
        update_playlist(&env.api, id, "通勤歌单", Some("上班路上听")).await.unwrap();
        update_playlist(&env.api, id, "通勤歌单", Some("  ")).await.unwrap();
        assert_eq!(checks(&env.pool, user.uid).await.len(), 2);

        // A playlist passes when the check fails
        update_playlist(&env.api, id, "名字#down", None).await.unwrap();
        assert_eq!(playlists(&env.api).await[0].name, "名字#down");
    }).await
}

#[tokio::test]
async fn playlist_covers_are_checked() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;
        let id = create_playlist(&env.api, "通勤歌单", None).await.unwrap();

        let uploaded = png(GREY);
        set_cover(&env.api, id, uploaded.clone()).await.unwrap();
        let cover_url = playlists(&env.api).await[0].cover_url.clone().expect("the cover should be saved");

        assert_eq!(set_cover(&env.api, id, png(RED)).await, Err("cover_rejected".to_string()));
        assert_eq!(playlists(&env.api).await[0].cover_url.as_ref(), Some(&cover_url));

        let covers: Vec<_> = checks(&env.pool, user.uid).await.into_iter().filter(|c| c.subject == "playlist.cover").collect();
        assert_eq!(covers.len(), 2);
        assert_eq!(covers[0].target_id, id);
        assert_eq!(covers[0].content, key_of(&cover_url));
        assert_eq!(covers[0].content_hash, hex::encode(Sha256::digest(&uploaded)));
        assert_eq!(covers[0].model.as_deref(), Some(TEST_IMAGE_MODEL));
        assert_eq!(covers[1].blocked_by, ["sexual"]);
        assert!(covers[1].content.starts_with("content-check/blocked/playlist/"), "{}", covers[1].content);
    }).await
}

#[tokio::test]
async fn cover_checks_are_limited_apart_from_avatars() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;
        let id = create_playlist(&env.api, "通勤歌单", None).await.unwrap();

        for _ in 0..10 {
            sqlx::query(
                "INSERT INTO content_checks (subject, target_id, actor_uid, content, content_hash, policy_version, blocked_by, latency_ms, create_time)
                 VALUES ('playlist.cover', $2, $1, 'earlier', 'earlier', 'earlier', '{}', 0, now() - interval '30 minutes')"
            ).bind(user.uid).bind(id).execute(&env.pool).await.unwrap();
        }
        assert_eq!(set_cover(&env.api, id, png(GREY)).await, Err("rate_limited".to_string()));

        // Avatars have their own limit
        set_avatar(&env.api, png(GREY)).await.unwrap();
    }).await
}

#[tokio::test]
async fn playlists_are_not_checked_without_content_check() {
    with_test_environment(|mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        let id = create_playlist(&env.api, "坏名#sexual", Some("加群领福利 #spam")).await.unwrap();
        set_cover(&env.api, id, png(RED)).await.unwrap();

        assert!(playlists(&env.api).await[0].cover_url.is_some());
        assert!(checks(&env.pool, user.uid).await.is_empty());
    }).await
}
