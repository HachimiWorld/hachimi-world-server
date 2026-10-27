use hachimi_world_server::db::comment::{Comment, CommentDao, CommentId, STATUS_PUBLISHED};
use hachimi_world_server::db::song::SongId;
use hachimi_world_server::db::user::UserId;
mod common;

use crate::common::auth::{with_new_random_test_user, with_test_contributor_user, TestUser};
use crate::common::publish::create_approved_song;
use crate::common::typesafe::{get_mock_typesafe_client, get_recovered_typesafe_client, new_content_checker};
use crate::common::{with_test_environment, with_test_environment_with, CommonParse, TestEnvironment, TestOptions};
use chrono::{Duration, Utc};
use hachimi_world_server::db::content_check::ContentCheckDao;
use hachimi_world_server::db::content_review::ContentReviewDao;
use hachimi_world_server::service::comment::{self, CommentItem};
use hachimi_world_server::service::content_review;
use hachimi_world_server::util::redlock::RedLock;
use hachimi_world_server::web::routes::comment::{CommentIdReq, CreateReq, ListReq, ListResp, RepliesReq, RepliesResp};
use hachimi_world_server::web::routes::message::{MarkReadReq, MarkReadResp, ReceivedRepliesReq, ReceivedRepliesResp, SummaryResp};
use hachimi_world_server::service::ugc::{CommentPreview, UgcPreview};
use hachimi_world_server::web::routes::report::{CaseReq, CaseResp, ResolveReq, ResolveResp, SubmitReq, SubmitResp};

const CHECKS_ON: TestOptions = TestOptions { content_check: true };

fn login(env: &mut TestEnvironment, user: &TestUser) {
    env.api.set_token(user.token.access_token.clone());
}

/// A song uploaded by a new user and approved by `contributor`, and that user.
async fn new_song(env: &mut TestEnvironment, contributor: &TestUser) -> (SongId, TestUser) {
    let owner = with_new_random_test_user(env).await;
    let song = create_approved_song(env, &owner.token.access_token, &contributor.token.access_token, "Commented").await;
    (song.id, owner)
}

async fn post(env: &TestEnvironment, song_id: SongId, content: &str, reply_to_id: Option<CommentId>) -> Result<CommentItem, String> {
    env.api.post("/comment/create", &CreateReq {
        target_type: "song".into(),
        target_id: song_id.0,
        content: content.into(),
        reply_to_id,
    }).await.parse_resp().await.map_err(|e| e.code)
}

async fn list(env: &TestEnvironment, song_id: SongId, sort: &str) -> Result<ListResp, String> {
    env.api.get_query("/comment/list", &ListReq {
        target_type: "song".into(),
        target_id: song_id.0,
        sort: Some(sort.into()),
        page_index: None,
        page_size: None,
    }).await.parse_resp().await.map_err(|e| e.code)
}

async fn replies(env: &TestEnvironment, root_id: CommentId) -> RepliesResp {
    env.api.get_query("/comment/replies", &RepliesReq { root_id, page_index: None, page_size: None }).await
        .parse_resp().await.unwrap()
}

async fn act(env: &TestEnvironment, action: &str, comment_id: CommentId) -> Result<(), String> {
    env.api.post(&format!("/comment/{action}"), &CommentIdReq { comment_id }).await
        .parse_resp::<()>().await.map_err(|e| e.code)
}

/// Top-level comment ids in the order listed.
fn root_ids(resp: &ListResp) -> Vec<CommentId> {
    resp.threads.iter().map(|x| x.comment.id).collect()
}

/// Redis locks are released asynchronously; wait before posting as the same user again.
async fn wait_lock() {
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
async fn comments_replies_and_likes() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let (song_id, owner) = new_song(&mut env, &contributor).await;
        let alice = with_new_random_test_user(&mut env).await;
        let bob = with_new_random_test_user(&mut env).await;

        login(&mut env, &alice);
        let first = post(&env, song_id, "  好听  ", None).await.unwrap();
        assert_eq!(first.content, "好听");
        assert_eq!(first.status, STATUS_PUBLISHED);
        assert_eq!(first.author.as_ref().unwrap().uid, alice.uid);
        wait_lock().await;
        let second = post(&env, song_id, "曼波", None).await.unwrap();

        login(&mut env, &bob);
        let reply = post(&env, song_id, "同意", Some(first.id)).await.unwrap();
        assert_eq!(reply.root_id, Some(first.id));
        assert_eq!(reply.reply_to_user.as_ref().map(|x| x.uid), None, "a reply to the top-level comment names no one");
        login(&mut env, &alice);
        let answer = post(&env, song_id, "谢谢", Some(reply.id)).await.unwrap();
        assert_eq!(answer.root_id, Some(first.id), "a reply to a reply stays under the same comment");
        assert_eq!(answer.reply_to_user.as_ref().map(|x| x.uid), Some(bob.uid));

        // Newest first, then most liked first
        let resp = list(&env, song_id, "new").await.unwrap();
        assert_eq!(root_ids(&resp), [second.id, first.id]);
        assert_eq!(resp.total, 4);
        let thread = &resp.threads[1];
        assert_eq!(thread.reply_count, 2);
        assert_eq!(thread.replies.iter().map(|x| x.id).collect::<Vec<_>>(), [reply.id, answer.id]);

        login(&mut env, &bob);
        act(&env, "like", first.id).await.unwrap();
        act(&env, "like", first.id).await.unwrap();
        let resp = list(&env, song_id, "hot").await.unwrap();
        assert_eq!(root_ids(&resp), [first.id, second.id]);
        assert_eq!(resp.threads[0].comment.like_count, 1, "liking twice counts once");
        assert!(resp.threads[0].comment.liked);
        assert!(!resp.threads[1].comment.liked);
        act(&env, "unlike", first.id).await.unwrap();
        act(&env, "unlike", first.id).await.unwrap();
        assert_eq!(list(&env, song_id, "hot").await.unwrap().threads[0].comment.like_count, 0);

        // Anyone can read without signing in
        env.api.clear_token();
        let replies = replies(&env, first.id).await;
        assert_eq!(replies.total, 2);
        assert!(!replies.replies[0].liked);
        let resp = env.api.post("/comment/create", &CreateReq {
            target_type: "song".into(), target_id: song_id.0, content: "匿名".into(), reply_to_id: None,
        }).await;
        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

        // Others can't delete; the author and the song's owner can
        login(&mut env, &bob);
        assert_eq!(act(&env, "delete", second.id).await.unwrap_err(), "permission_denied");
        login(&mut env, &owner);
        act(&env, "delete", second.id).await.unwrap();
        login(&mut env, &alice);
        act(&env, "delete", first.id).await.unwrap();
        assert_eq!(act(&env, "delete", first.id).await.unwrap_err(), "comment_not_found");

        // A deleted comment with replies keeps its place, without its author or text
        let resp = list(&env, song_id, "new").await.unwrap();
        assert_eq!(root_ids(&resp), [first.id]);
        let removed = &resp.threads[0].comment;
        assert!(removed.removed);
        assert_eq!(removed.content, "");
        assert!(removed.author.is_none());
        assert_eq!(resp.threads[0].reply_count, 2);
        assert_eq!(resp.total, 2);
        // And can't be replied to or liked any more
        assert_eq!(post(&env, song_id, "还在吗", Some(first.id)).await.unwrap_err(), "comment_not_found");
        assert_eq!(act(&env, "like", first.id).await.unwrap_err(), "comment_not_found");
    }).await
}

#[tokio::test]
async fn threads_show_their_first_replies() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let (song_id, _owner) = new_song(&mut env, &contributor).await;
        let alice = with_new_random_test_user(&mut env).await;

        login(&mut env, &alice);
        let busy = post(&env, song_id, "很多回复", None).await.unwrap();
        wait_lock().await;
        let quiet = post(&env, song_id, "一条回复", None).await.unwrap();
        let mut busy_replies = vec![];
        for i in 0..5 {
            wait_lock().await;
            busy_replies.push(post(&env, song_id, &format!("回复 {i}"), Some(busy.id)).await.unwrap().id);
        }
        wait_lock().await;
        let quiet_reply = post(&env, song_id, "就这一条", Some(quiet.id)).await.unwrap();

        // Each thread shows its own first 3 replies, oldest first
        let resp = list(&env, song_id, "new").await.unwrap();
        assert_eq!(root_ids(&resp), [quiet.id, busy.id]);
        assert_eq!(resp.threads[0].replies.iter().map(|x| x.id).collect::<Vec<_>>(), [quiet_reply.id]);
        assert_eq!(resp.threads[0].reply_count, 1);
        assert_eq!(resp.threads[1].replies.iter().map(|x| x.id).collect::<Vec<_>>(), busy_replies[..3]);
        assert_eq!(resp.threads[1].reply_count, 5);
    }).await;
}

#[tokio::test]
async fn invalid_comments_are_refused() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let (song_id, _) = new_song(&mut env, &contributor).await;
        let (other_song_id, _) = new_song(&mut env, &contributor).await;
        let user = with_new_random_test_user(&mut env).await;
        login(&mut env, &user);

        assert_eq!(post(&env, song_id, "   ", None).await.unwrap_err(), "comment_empty");
        assert_eq!(post(&env, song_id, &"哈".repeat(501), None).await.unwrap_err(), "comment_too_long");
        post(&env, song_id, &"哈".repeat(500), None).await.unwrap();
        assert_eq!(post(&env, SongId(i64::MAX), "好听", None).await.unwrap_err(), "target_not_found");
        let resp = env.api.post("/comment/create", &CreateReq {
            target_type: "user".into(), target_id: user.uid.0, content: "好听".into(), reply_to_id: None,
        }).await.parse_resp::<CommentItem>().await.unwrap_err();
        assert_eq!(resp.code, "invalid_target_type");
        assert_eq!(list(&env, song_id, "old").await.unwrap_err(), "invalid_sort");

        // A reply must be to a comment on the same song
        wait_lock().await;
        let comment = post(&env, song_id, "好听", None).await.unwrap();
        wait_lock().await;
        assert_eq!(post(&env, other_song_id, "好听", Some(comment.id)).await.unwrap_err(), "comment_not_found");

        // Banned users can't comment
        sqlx::query("UPDATE users SET is_banned = TRUE WHERE id = $1").bind(user.uid).execute(&env.pool).await.unwrap();
        assert_eq!(post(&env, song_id, "好听", None).await.unwrap_err(), "user_banned");
    }).await
}

#[tokio::test]
async fn hidden_songs_hide_their_comments() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let (song_id, owner) = new_song(&mut env, &contributor).await;
        let user = with_new_random_test_user(&mut env).await;
        login(&mut env, &user);
        post(&env, song_id, "好听", None).await.unwrap();

        sqlx::query("UPDATE songs SET is_hidden = TRUE WHERE id = $1").bind(song_id).execute(&env.pool).await.unwrap();
        assert_eq!(list(&env, song_id, "hot").await.unwrap_err(), "target_not_found");
        wait_lock().await;
        assert_eq!(post(&env, song_id, "还在吗", None).await.unwrap_err(), "target_not_found");
        login(&mut env, &owner);
        assert_eq!(list(&env, song_id, "hot").await.unwrap().threads.len(), 1);
    }).await
}

#[tokio::test]
async fn comments_are_limited_per_hour() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let (song_id, _) = new_song(&mut env, &contributor).await;
        let user = with_new_random_test_user(&mut env).await;
        login(&mut env, &user);
        let now = Utc::now();
        for _ in 0..29 {
            CommentDao::insert(&env.pool, &Comment {
                id: CommentId(0),
                target_type: "song".into(),
                target_id: song_id.0,
                user_id: user.uid,
                root_id: None,
                reply_to_id: None,
                reply_to_uid: user.uid,
                content: "刷屏".into(),
                status: STATUS_PUBLISHED.into(),
                is_hidden: false,
                like_count: 0,
                // Deleted ones count too
                delete_time: Some(now),
                create_time: now,
                update_time: now,
            }).await.unwrap();
        }
        post(&env, song_id, "第三十条", None).await.unwrap();
        wait_lock().await;
        assert_eq!(post(&env, song_id, "第三十一条", None).await.unwrap_err(), "rate_limited");
    }).await
}

async fn summary(env: &TestEnvironment) -> SummaryResp {
    env.api.get("/message/summary").await.parse_resp().await.unwrap()
}

async fn received_replies(env: &TestEnvironment) -> ReceivedRepliesResp {
    env.api.get_query("/message/received_replies", &ReceivedRepliesReq { before_time: None, before_comment_id: None, limit: None }).await
        .parse_resp().await.unwrap()
}

#[tokio::test]
async fn replies_reach_the_message_center() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let (song_id, owner) = new_song(&mut env, &contributor).await;
        let alice = with_new_random_test_user(&mut env).await;
        let bob = with_new_random_test_user(&mut env).await;

        login(&mut env, &owner);
        post(&env, song_id, "欢迎评论", None).await.unwrap();
        login(&mut env, &alice);
        let comment = post(&env, song_id, "好听", None).await.unwrap();
        login(&mut env, &bob);
        let reply = post(&env, song_id, "同意", Some(comment.id)).await.unwrap();
        login(&mut env, &alice);
        wait_lock().await;
        post(&env, song_id, "自己回自己", Some(comment.id)).await.unwrap();

        // The owner gets comments on their song, but not their own
        login(&mut env, &owner);
        assert_eq!(summary(&env).await.reply_unread, 1);
        let items = received_replies(&env).await.items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].comment.id, comment.id);
        assert_eq!(items[0].replied_content, None);
        assert_eq!(items[0].target.target_id, song_id.0);
        assert!(matches!(&items[0].target.preview, UgcPreview::Song(song) if song.title.starts_with("Commented-")));

        // Alice gets Bob's reply, but not her own
        login(&mut env, &alice);
        let summary_before = summary(&env).await;
        assert_eq!(summary_before.reply_unread, 1);
        let items = received_replies(&env).await.items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].comment.id, reply.id);
        assert_eq!(items[0].replied_content.as_deref(), Some("好听"));

        let read: MarkReadResp = env.api.post("/message/mark_read", &MarkReadReq { channel: "reply".into() }).await.parse_resp().await.unwrap();
        let summary_after = summary(&env).await;
        assert_eq!(summary_after.reply_unread, 0);
        assert_eq!(summary_after.reply_read_time, read.read_time);
        assert_eq!(summary_after.total_unread, summary_before.total_unread - 1);
        // Read replies are still listed
        assert_eq!(received_replies(&env).await.items.len(), 1);

        // A deleted reply isn't
        login(&mut env, &bob);
        act(&env, "delete", reply.id).await.unwrap();
        login(&mut env, &alice);
        assert!(received_replies(&env).await.items.is_empty());
    }).await
}

#[tokio::test]
async fn unread_replies_are_counted_up_to_a_hundred() {
    with_test_environment(|env| async move {
        let (owner, other) = (UserId(1), UserId(2));
        sqlx::query(
            "INSERT INTO comments (target_type, target_id, user_id, reply_to_uid, content, status, is_hidden, like_count, create_time, update_time)
            SELECT 'song', 1, $2, $1, 'x', 'published', false, 0, now(), now() FROM generate_series(1, 150)"
        ).bind(owner.0).bind(other.0).execute(&env.pool).await.unwrap();

        let since = Utc::now() - Duration::days(1);
        assert_eq!(comment::count_received_replies_after(&env.pool, owner, since).await.unwrap(), comment::MAX_UNREAD_REPLIES);
        assert_eq!(comment::count_received_replies_after(&env.pool, other, since).await.unwrap(), 0);
    }).await
}

#[tokio::test]
async fn reported_comments_can_be_hidden() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let (song_id, _) = new_song(&mut env, &contributor).await;
        let author = with_new_random_test_user(&mut env).await;
        let reporter = with_new_random_test_user(&mut env).await;

        login(&mut env, &author);
        let comment = post(&env, song_id, "广告", None).await.unwrap();
        login(&mut env, &reporter);
        let report: SubmitResp = env.api.post("/report/submit", &SubmitReq {
            target_type: "comment".into(), target_id: comment.id.0, reason: "spam".into(), detail: None,
        }).await.parse_resp().await.unwrap();
        wait_lock().await;

        login(&mut env, &contributor);
        let detail: CaseResp = env.api.get_query("/report/case", &CaseReq { target_type: "comment".into(), target_id: comment.id.0 }).await
            .parse_resp().await.unwrap();
        let target = detail.case.target.unwrap();
        let UgcPreview::Comment(CommentPreview { content, on: Some(on) }) = target.preview else { panic!("Not a comment on something: {:?}", target.preview) };
        assert_eq!(content, "广告");
        assert!(matches!(*on, UgcPreview::Song(ref song) if song.title.starts_with("Commented-")));
        assert_eq!(target.owner.unwrap().uid, author.uid);

        let _: ResolveResp = env.api.post("/report/resolve", &ResolveReq {
            target_type: "comment".into(),
            target_id: comment.id.0,
            verdict: "agree".into(),
            note: None,
            content_actions: vec!["hide".into()],
            author_reason: Some("广告".into()),
            ignore_reports: false,
            up_to_report_id: report.report_id,
        }).await.parse_resp().await.unwrap();

        login(&mut env, &reporter);
        assert!(list(&env, song_id, "hot").await.unwrap().threads.is_empty());
        assert_eq!(act(&env, "like", comment.id).await.unwrap_err(), "comment_not_found");
        login(&mut env, &author);
        let threads = list(&env, song_id, "hot").await.unwrap().threads;
        assert_eq!(threads.len(), 1);
        assert!(threads[0].comment.is_hidden);

        let (title, intent): (String, serde_json::Value) = sqlx::query_as(
            "SELECT title, content_intent FROM notifications WHERE recipient_uid = $1 AND notification_type = 'governance.content_hidden'"
        ).bind(author.uid).fetch_one(&env.pool).await.unwrap();
        assert_eq!(title, "评论已被隐藏");
        assert_eq!(intent["action"], "song.comment.view");
        assert_eq!(intent["data"]["comment_id"], comment.id.0);
        assert!(intent["data"]["song_display_id"].as_str().is_some_and(|x| !x.is_empty()));
    }).await
}

async fn check_count(env: &TestEnvironment, uid: UserId) -> usize {
    ContentCheckDao::list_by_actor(&env.pool, uid, 100).await.unwrap().len()
}

#[tokio::test]
async fn comments_are_checked() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let (song_id, _) = new_song(&mut env, &contributor).await;
        let author = with_new_random_test_user(&mut env).await;
        let other = with_new_random_test_user(&mut env).await;
        login(&mut env, &author);

        let comment = post(&env, song_id, "好听", None).await.unwrap();
        assert_eq!(comment.status, "published");
        let checks = ContentCheckDao::list_by_actor(&env.pool, author.uid, 10).await.unwrap();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].subject, "song.comment");
        assert_eq!(checks[0].target_id, comment.id.0);

        // A blocked comment isn't saved
        wait_lock().await;
        assert_eq!(post(&env, song_id, "#sexual", None).await.unwrap_err(), "comment_rejected");
        assert_eq!(list(&env, song_id, "new").await.unwrap().threads.len(), 1);
        assert_eq!(check_count(&env, author.uid).await, 2);

        // Blocked tries count towards the limit, as rewording to get past the check costs too
        let now = Utc::now();
        for _ in 0..28 {
            sqlx::query("INSERT INTO content_checks (subject, target_id, actor_uid, content, content_hash, policy_version, model, answers, error, blocked_by, latency_ms, reused_from, create_time)
                VALUES ('song.comment', 0, $1, '#sexual', 'x', 'x', 'jev-test', NULL, NULL, '{sexual}', 0, NULL, $2)")
                .bind(author.uid).bind(now).execute(&env.pool).await.unwrap();
        }
        wait_lock().await;
        assert_eq!(post(&env, song_id, "好听", None).await.unwrap_err(), "rate_limited");

        // Another user can't see a comment that is waiting for its check
        login(&mut env, &other);
        let pending = post(&env, song_id, "#down 好听", None).await.unwrap();
        assert_eq!(pending.status, "pending");
        let mine = list(&env, song_id, "new").await.unwrap();
        assert_eq!(mine.threads[0].comment.id, pending.id);
        assert_eq!(mine.threads[0].comment.status, "pending");
        assert_eq!(mine.total, 1, "the total counts only published comments");
        login(&mut env, &author);
        assert_eq!(root_ids(&list(&env, song_id, "new").await.unwrap()), [comment.id]);
        assert_eq!(act(&env, "like", pending.id).await.unwrap_err(), "comment_not_found");
        assert_eq!(post(&env, song_id, "回复", Some(pending.id)).await.unwrap_err(), "comment_not_found");
    }).await
}

/// Runs the content reviews due at `now` with `checker`.
async fn run_reviews(env: &TestEnvironment, client: hachimi_world_server::util::typesafe::MockTypeSafeClient, now: chrono::DateTime<Utc>) -> usize {
    let checker = new_content_checker(&env.pool, client).await.unwrap();
    let red_lock = RedLock::new(env.redis.clone()).unwrap();
    content_review::run(&env.pool, &red_lock, Some(&checker), now).await.unwrap()
}

async fn status(env: &TestEnvironment, id: CommentId) -> String {
    CommentDao::get_by_id(&env.pool, id).await.unwrap().unwrap().status
}

#[tokio::test]
async fn waiting_comments_are_checked_again() {
    with_test_environment_with(CHECKS_ON, |mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let (song_id, _) = new_song(&mut env, &contributor).await;
        let author = with_new_random_test_user(&mut env).await;
        login(&mut env, &author);
        let good = post(&env, song_id, "#down 好听", None).await.unwrap();
        wait_lock().await;
        let bad = post(&env, song_id, "#down #sexual", None).await.unwrap();
        wait_lock().await;
        let deleted = post(&env, song_id, "#down 删了", None).await.unwrap();
        act(&env, "delete", deleted.id).await.unwrap();
        assert_eq!(good.status, "pending");

        // Not due yet
        let now = Utc::now();
        assert_eq!(run_reviews(&env, get_recovered_typesafe_client(), now).await, 0);

        // TypeSafe still down: tried again later
        assert_eq!(run_reviews(&env, get_mock_typesafe_client(), now + Duration::minutes(2)).await, 3);
        assert_eq!(status(&env, good.id).await, "pending");
        let review = ContentReviewDao::list_by_subject_and_target(&env.pool, "song.comment", good.id.0).await.unwrap().remove(0);
        assert_eq!(review.attempt_count, 1);
        assert!(review.done_time.is_none());
        assert!(review.available_time > now + Duration::minutes(6));

        // Back up: settled either way
        wait_lock().await;
        assert_eq!(run_reviews(&env, get_recovered_typesafe_client(), now + Duration::minutes(10)).await, 2);
        assert_eq!(status(&env, good.id).await, "published");
        assert_eq!(status(&env, bad.id).await, "rejected");
        assert_eq!(status(&env, deleted.id).await, "pending");
        wait_lock().await;
        assert_eq!(run_reviews(&env, get_recovered_typesafe_client(), now + Duration::days(2)).await, 0);

        let mine = list(&env, song_id, "new").await.unwrap();
        assert_eq!(root_ids(&mine), [bad.id, good.id]);
        assert_eq!(mine.threads[0].comment.status, "rejected");
        assert_eq!(mine.total, 1);
    }).await
}

#[tokio::test]
async fn comments_that_cant_be_checked_for_a_day_are_published() {
    // The test runs on one thread, so the worker records here
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let _guard = metrics::set_default_local_recorder(&recorder);
    with_test_environment_with(CHECKS_ON, |mut env| { let metrics = handle.clone(); async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let (song_id, _) = new_song(&mut env, &contributor).await;
        let author = with_new_random_test_user(&mut env).await;
        login(&mut env, &author);
        let comment = post(&env, song_id, "#down 好听", None).await.unwrap();

        let now = Utc::now();
        run_reviews(&env, get_mock_typesafe_client(), now + Duration::hours(23)).await;
        assert_eq!(status(&env, comment.id).await, "pending");
        content_review::report_pending(&env.pool, now + Duration::hours(23)).await.unwrap();
        let out = metrics.render();
        assert!(out.contains(r#"content_review_pending{subject="song.comment"} 1"#), "{out}");
        let oldest = gauge_value(&out, r#"content_review_oldest_pending_seconds{subject="song.comment"}"#);
        assert!((oldest - 23.0 * 3600.0).abs() < 60.0, "{out}");

        wait_lock().await;
        run_reviews(&env, get_mock_typesafe_client(), now + Duration::hours(25)).await;
        assert_eq!(status(&env, comment.id).await, "published");
        let review = ContentReviewDao::list_by_subject_and_target(&env.pool, "song.comment", comment.id.0).await.unwrap().remove(0);
        assert!(review.done_time.is_some());
        content_review::report_pending(&env.pool, now + Duration::hours(25)).await.unwrap();
        let out = metrics.render();
        assert!(out.contains(r#"content_review_settled_total{subject="song.comment",outcome="gave_up"} 1"#), "{out}");
        assert!(out.contains(r#"content_review_pending{subject="song.comment"} 0"#), "{out}");
        assert!(out.contains(r#"content_review_oldest_pending_seconds{subject="song.comment"} 0"#), "{out}");
    }}).await
}

fn gauge_value(rendered: &str, series: &str) -> f64 {
    rendered.lines()
        .find_map(|x| x.strip_prefix(series))
        .unwrap_or_else(|| panic!("{series} not in {rendered}"))
        .trim().parse().unwrap()
}
