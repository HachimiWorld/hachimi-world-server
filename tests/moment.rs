mod common;

use crate::common::auth::{with_new_random_test_user, with_test_contributor_user, TestUser};
use crate::common::publish::create_approved_song;
use crate::common::typesafe::{get_mock_typesafe_client, get_recovered_typesafe_client, new_content_checker};
use crate::common::{with_test_environment, with_test_environment_with, CommonParse, TestEnvironment, TestOptions};
use chrono::{Duration, Utc};
use hachimi_world_server::db::moment::{MomentDao, MomentId};
use hachimi_world_server::db::playlist::PlaylistId;
use hachimi_world_server::db::song::SongId;
use hachimi_world_server::service::content_review;
use hachimi_world_server::service::moment::{AttachmentInput, MomentItem};
use hachimi_world_server::service::ugc::UgcPreview;
use hachimi_world_server::util::redlock::RedLock;
use hachimi_world_server::web::routes::comment::{CreateReq as CommentCreateReq, CreateResp as CommentCreateResp};
use hachimi_world_server::web::routes::feed::{PageReq, PageResp as FeedPageResp};
use hachimi_world_server::web::routes::moment::{CreateReq, CreateResp, MomentIdReq, PageByUserReq, PageResp, UploadImageResp};
use hachimi_world_server::web::routes::playlist::{CreatePlaylistReq, CreatePlaylistResp};
use hachimi_world_server::web::routes::report::{CaseReq, CaseResp, ResolveReq, ResolveResp, SubmitReq, SubmitResp};
use image::{ImageFormat, Rgb, RgbImage};
use reqwest::multipart::{Form, Part};
use serde_json::json;

const CHECKS_ON: TestOptions = TestOptions { content_check: true };

const GREY: [u8; 3] = [128, 128, 128];
/// Blocked as `sexual` by the mock
const RED: [u8; 3] = [230, 20, 20];
/// Fails the mock's call
const BLUE: [u8; 3] = [20, 20, 230];

fn login(env: &mut TestEnvironment, user: &TestUser) {
    env.api.set_token(user.token.access_token.clone());
}

/// Redis locks are released asynchronously; wait before posting as the same user again.
async fn wait_lock() {
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

/// A PNG of one colour, `width` × `height`; the mock judges images by their colour.
fn png(color: [u8; 3], width: u32, height: u32) -> Vec<u8> {
    let mut png = Vec::new();
    RgbImage::from_pixel(width, height, Rgb(color)).write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png).unwrap();
    png
}

async fn upload(env: &TestEnvironment, bytes: Vec<u8>) -> Result<UploadImageResp, String> {
    env.api.post_raw("/moment/upload_image")
        .multipart(Form::new().part("file", Part::bytes(bytes).file_name("image.png")))
        .send().await.unwrap()
        .parse_resp().await.map_err(|e| e.code)
}

/// Posts a moment; `None` if it waits for content checks.
async fn try_create(env: &TestEnvironment, content: &str, image_ids: Vec<String>, attachments: Vec<AttachmentInput>) -> Result<Option<MomentItem>, String> {
    env.api.post("/moment/create", &CreateReq { content: content.into(), image_ids, attachments }).await
        .parse_resp::<CreateResp>().await.map(|x| x.moment).map_err(|e| e.code)
}

async fn create(env: &TestEnvironment, content: &str) -> MomentItem {
    try_create(env, content, vec![], vec![]).await.unwrap().expect("Published at once")
}

fn attach(kind: &str, id: i64) -> AttachmentInput {
    AttachmentInput { kind: kind.into(), data: json!({ "id": id }) }
}

async fn detail(env: &TestEnvironment, moment_id: MomentId) -> Result<MomentItem, String> {
    env.api.get_query("/moment/detail", &MomentIdReq { moment_id }).await.parse_resp().await.map_err(|e| e.code)
}

async fn act(env: &TestEnvironment, action: &str, moment_id: MomentId) -> Result<(), String> {
    env.api.post(&format!("/moment/{action}"), &MomentIdReq { moment_id }).await.parse_resp::<()>().await.map_err(|e| e.code)
}

async fn square_page(env: &TestEnvironment, cursor: Option<String>, limit: Option<i64>) -> FeedPageResp {
    env.api.get_query("/feed/square", &PageReq { cursor, limit }).await.parse_resp().await.unwrap()
}

/// Moment ids on the first page of the square, in order.
async fn square(env: &TestEnvironment) -> Vec<MomentId> {
    square_page(env, None, None).await.items.into_iter().map(|x| {
        assert_eq!(x.kind, "moment");
        x.moment.unwrap().id
    }).collect()
}

async fn by_user(env: &TestEnvironment, user: &TestUser) -> PageResp {
    env.api.get_query("/moment/page_by_user", &PageByUserReq { user_id: user.uid, before_time: None, before_id: None, limit: None }).await
        .parse_resp().await.unwrap()
}

async fn new_song(env: &mut TestEnvironment) -> SongId {
    let contributor = with_test_contributor_user(env).await;
    let owner = with_new_random_test_user(env).await;
    create_approved_song(env, &owner.token.access_token, &contributor.token.access_token, "Shared").await.id
}

async fn new_playlist(env: &TestEnvironment, is_public: bool) -> PlaylistId {
    env.api.post("/playlist/create", &CreatePlaylistReq { name: "分享的歌单".into(), description: None, is_public }).await
        .parse_resp::<CreatePlaylistResp>().await.unwrap().id
}

async fn run_reviews(env: &TestEnvironment, client: hachimi_world_server::util::typesafe::MockTypeSafeClient, now: chrono::DateTime<Utc>) -> usize {
    let checker = new_content_checker(&env.pool, client).await.unwrap();
    let red_lock = RedLock::new(env.redis.clone()).unwrap();
    content_review::run(&env.pool, &red_lock, Some(&checker), env.file_host.as_ref(), now).await.unwrap()
}

/// The id the moment `uid` posted last that waits for content checks will have.
async fn last_review_target(env: &TestEnvironment, user: &TestUser) -> MomentId {
    let id: i64 = sqlx::query_scalar("SELECT target_id FROM content_reviews WHERE actor_uid = $1 ORDER BY id DESC LIMIT 1")
        .bind(user.uid).fetch_one(&env.pool).await.unwrap();
    MomentId(id)
}

#[tokio::test]
async fn moments_with_images_and_attachments() {
    with_test_environment(|mut env| async move {
        let song_id = new_song(&mut env).await;
        let alice = with_new_random_test_user(&mut env).await;
        let bob = with_new_random_test_user(&mut env).await;

        login(&mut env, &alice);
        let wide = upload(&env, png(GREY, 3000, 1000)).await.unwrap();
        assert_eq!((wide.width, wide.height), (2048, 683), "scaled down to fit 2048, keeping its shape");
        let small = upload(&env, png([100, 150, 100], 300, 200)).await.unwrap();
        assert_eq!((small.width, small.height), (300, 200));
        assert_ne!(wide.url, wide.thumb_url);

        let first = try_create(&env, "  今天听到一首好歌  ", vec![wide.image_id.clone(), small.image_id.clone()], vec![attach("song", song_id.0)]).await
            .unwrap().unwrap();
        assert_eq!(first.content, "今天听到一首好歌");
        assert_eq!(first.author.uid, alice.uid);
        assert_eq!(first.images.iter().map(|x| &x.url).collect::<Vec<_>>(), [&wide.url, &small.url]);
        assert_eq!(first.attachments.len(), 1);
        assert_eq!(first.attachments[0].kind, "song");
        assert_eq!(first.attachments[0].data.as_ref().unwrap()["id"], song_id.0);
        // Posted images can't be posted again
        wait_lock().await;
        assert_eq!(try_create(&env, "再发一次", vec![wide.image_id], vec![]).await.unwrap_err(), "image_not_found");

        let playlist_id = new_playlist(&env, true).await;
        let second = try_create(&env, "", vec![], vec![attach("playlist", playlist_id.0)]).await.unwrap().unwrap();
        assert_eq!(second.attachments[0].data.as_ref().unwrap()["name"], "分享的歌单");

        // Everyone sees them, newest first
        login(&mut env, &bob);
        assert_eq!(square(&env).await, [second.id, first.id]);
        assert_eq!(by_user(&env, &alice).await.moments.len(), 2);

        // Likes and comments
        act(&env, "like", first.id).await.unwrap();
        act(&env, "like", first.id).await.unwrap();
        let shown = detail(&env, first.id).await.unwrap();
        assert_eq!((shown.like_count, shown.liked), (1, true));
        let _: CommentCreateResp = env.api.post("/comment/create", &CommentCreateReq {
            target_type: "moment".into(), target_id: first.id.0, content: "确实好听".into(), reply_to_id: None,
        }).await.parse_resp().await.unwrap();
        assert_eq!(detail(&env, first.id).await.unwrap().comment_count, 1);
        act(&env, "unlike", first.id).await.unwrap();
        let shown = detail(&env, first.id).await.unwrap();
        assert_eq!((shown.like_count, shown.liked), (0, false));

        // A playlist made private since shows as unavailable
        login(&mut env, &alice);
        sqlx::query("UPDATE playlists SET is_public = false WHERE id = $1").bind(playlist_id.0).execute(&env.pool).await.unwrap();
        assert!(detail(&env, second.id).await.unwrap().attachments[0].data.is_none());

        // Only its author can delete it, and then it's gone for everyone
        login(&mut env, &bob);
        assert_eq!(act(&env, "delete", first.id).await.unwrap_err(), "moment_not_found");
        login(&mut env, &alice);
        act(&env, "delete", first.id).await.unwrap();
        assert_eq!(detail(&env, first.id).await.unwrap_err(), "moment_not_found");
        assert_eq!(square(&env).await, [second.id]);
        assert_eq!(by_user(&env, &alice).await.moments.len(), 1);
    }).await
}

#[tokio::test]
async fn the_square_pages_by_cursor() {
    with_test_environment(|mut env| async move {
        let mut ids = vec![];
        for i in 0..3 {
            let user = with_new_random_test_user(&mut env).await;
            login(&mut env, &user);
            ids.push(create(&env, &format!("第 {i} 条")).await.id);
        }
        ids.reverse();
        env.api.clear_token();
        let first = square_page(&env, None, Some(2)).await;
        let cursor = first.next_cursor.clone().unwrap();
        let second = square_page(&env, Some(cursor), Some(2)).await;
        let listed: Vec<MomentId> = first.items.into_iter().chain(second.items).map(|x| x.moment.unwrap().id).collect();
        assert_eq!(listed, ids);
        assert!(second.next_cursor.is_none());
        let resp = env.api.get_query("/feed/square", &PageReq { cursor: Some("bad".into()), limit: None }).await
            .parse_resp::<FeedPageResp>().await;
        assert_eq!(resp.unwrap_err().code, "invalid_cursor");
    }).await
}

#[tokio::test]
async fn invalid_moments_are_refused() {
    with_test_environment(|mut env| async move {
        let alice = with_new_random_test_user(&mut env).await;
        let bob = with_new_random_test_user(&mut env).await;
        login(&mut env, &bob);
        let bobs_image = upload(&env, png(GREY, 300, 200)).await.unwrap();
        let private = new_playlist(&env, false).await;

        login(&mut env, &alice);
        assert_eq!(try_create(&env, "   ", vec![], vec![]).await.unwrap_err(), "moment_empty");
        assert_eq!(try_create(&env, &"字".repeat(501), vec![], vec![]).await.unwrap_err(), "content_too_long");
        assert_eq!(try_create(&env, "图", vec!["x".into(); 10], vec![]).await.unwrap_err(), "too_many_images");
        assert_eq!(try_create(&env, "两个", vec![], vec![attach("song", 1), attach("song", 2)]).await.unwrap_err(), "too_many_attachments");
        assert_eq!(try_create(&env, "投票", vec![], vec![attach("poll", 1)]).await.unwrap_err(), "unknown_attachment");
        assert_eq!(try_create(&env, "没这首歌", vec![], vec![attach("song", 999_999)]).await.unwrap_err(), "invalid_attachment");
        assert_eq!(try_create(&env, "别人的私密歌单", vec![], vec![attach("playlist", private.0)]).await.unwrap_err(), "invalid_attachment");
        assert_eq!(try_create(&env, "别人的图", vec![bobs_image.image_id], vec![]).await.unwrap_err(), "image_not_found");
        let resp = env.api.post_raw("/moment/upload_image")
            .multipart(Form::new().part("file", Part::bytes(b"not an image".to_vec()).file_name("x.png")))
            .send().await.unwrap().parse_resp::<UploadImageResp>().await;
        assert_eq!(resp.unwrap_err().code, "invalid_image");

        // Two a minute
        create(&env, "一").await;
        wait_lock().await;
        create(&env, "二").await;
        wait_lock().await;
        assert_eq!(try_create(&env, "三", vec![], vec![]).await.unwrap_err(), "rate_limited");

    }).await
}

#[tokio::test]
async fn images_and_text_are_checked() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let alice = with_new_random_test_user(&mut env).await;
        login(&mut env, &alice);

        // Blocked at upload, so it can be swapped before posting
        assert_eq!(upload(&env, png(RED, 300, 200)).await.unwrap_err(), "image_rejected");
        let fine = upload(&env, png(GREY, 300, 200)).await.unwrap();
        assert_eq!(try_create(&env, "#sexual", vec![fine.image_id.clone()], vec![]).await.unwrap_err(), "moment_rejected");
        // The image is still there to post
        wait_lock().await;
        let moment = try_create(&env, "正常的", vec![fine.image_id], vec![]).await.unwrap().unwrap();
        assert_eq!(moment.images.len(), 1);
    }).await
}

#[tokio::test]
async fn moments_that_cant_be_checked_wait() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let alice = with_new_random_test_user(&mut env).await;
        let bob = with_new_random_test_user(&mut env).await;
        let carol = with_new_random_test_user(&mut env).await;

        // TypeSafe is down for this image: kept, and checked again with the moment
        login(&mut env, &alice);
        let unchecked = upload(&env, png(BLUE, 300, 200)).await.unwrap();
        assert!(try_create(&env, "图审不了", vec![unchecked.image_id], vec![]).await.unwrap().is_none());
        let with_image = last_review_target(&env, &alice).await;
        // Fine once only `#down` is gone
        login(&mut env, &bob);
        assert!(try_create(&env, "#down 好听", vec![], vec![]).await.unwrap().is_none());
        let good = last_review_target(&env, &bob).await;
        // Blocked once TypeSafe is back
        login(&mut env, &carol);
        assert!(try_create(&env, "#down #sexual", vec![], vec![]).await.unwrap().is_none());
        let bad = last_review_target(&env, &carol).await;

        // Seen by no one while waiting, their authors included
        login(&mut env, &alice);
        assert!(square(&env).await.is_empty());
        assert!(by_user(&env, &alice).await.moments.is_empty());

        // Still down: tried again later
        let now = Utc::now();
        assert_eq!(run_reviews(&env, get_mock_typesafe_client(), now + Duration::minutes(2)).await, 3);
        assert!(MomentDao::get_by_id(&env.pool, good).await.unwrap().is_none());

        // Back up: saved if everything passes, dropped and the author told if not
        wait_lock().await;
        assert_eq!(run_reviews(&env, get_recovered_typesafe_client(), now + Duration::minutes(10)).await, 3);
        assert!(MomentDao::get_by_id(&env.pool, with_image).await.unwrap().is_some());
        assert!(MomentDao::get_by_id(&env.pool, good).await.unwrap().is_some());
        assert!(MomentDao::get_by_id(&env.pool, bad).await.unwrap().is_none());
        // In their place by when they were posted
        assert_eq!(square(&env).await, [good, with_image]);
        let told: Vec<String> = sqlx::query_scalar("SELECT body FROM notifications WHERE recipient_uid = $1 AND notification_type = 'moment.rejected'")
            .bind(carol.uid).fetch_all(&env.pool).await.unwrap();
        assert_eq!(told, ["你发布的动态「#down #sexual」没有通过审核，没有发出。"]);
    }).await
}

#[tokio::test]
async fn reported_moments_can_be_hidden() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let author = with_new_random_test_user(&mut env).await;
        let reporter = with_new_random_test_user(&mut env).await;
        login(&mut env, &author);
        let moment = create(&env, "广告").await;

        login(&mut env, &reporter);
        let report: SubmitResp = env.api.post("/report/submit", &SubmitReq {
            target_type: "moment".into(), target_id: moment.id.0, reason: "spam".into(), detail: None,
        }).await.parse_resp().await.unwrap();
        login(&mut env, &contributor);
        let case: CaseResp = env.api.get_query("/report/case", &CaseReq { target_type: "moment".into(), target_id: moment.id.0 }).await
            .parse_resp().await.unwrap();
        let target = case.case.target.unwrap();
        assert!(matches!(target.preview, UgcPreview::Moment(ref x) if x.content == "广告" && x.id == moment.id));
        let _: ResolveResp = env.api.post("/report/resolve", &ResolveReq {
            target_type: "moment".into(),
            target_id: moment.id.0,
            verdict: "agree".into(),
            note: None,
            content_actions: vec!["hide".into()],
            author_reason: Some("广告".into()),
            ignore_reports: false,
            up_to_report_id: report.report_id,
        }).await.parse_resp().await.unwrap();

        // Gone for others, still there for its author
        login(&mut env, &reporter);
        assert!(square(&env).await.is_empty());
        assert_eq!(detail(&env, moment.id).await.unwrap_err(), "moment_not_found");
        assert_eq!(act(&env, "like", moment.id).await.unwrap_err(), "moment_not_found");
        login(&mut env, &author);
        assert!(detail(&env, moment.id).await.unwrap().is_hidden);
        assert_eq!(by_user(&env, &author).await.moments.len(), 1);
        let body: String = sqlx::query_scalar("SELECT body FROM notifications WHERE recipient_uid = $1 AND notification_type = 'governance.content_hidden'")
            .bind(author.uid).fetch_one(&env.pool).await.unwrap();
        assert!(body.starts_with("你的动态「广告」已被隐藏"), "{body}");
    }).await
}
