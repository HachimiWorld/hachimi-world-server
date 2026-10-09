mod common;

use crate::common::auth::{with_new_random_test_user, with_test_contributor_user, TestUser};
use crate::common::publish::create_approved_song;
use crate::common::{CommonParse, TestEnvironment};
use chrono::{Duration, Utc};
use common::with_test_environment;
use hachimi_world_server::service::notification::{send_notification, NewNotification};
use hachimi_world_server::web::routes::message::{MarkReadReq, MarkReadResp, ReceivedLikesReq, ReceivedLikesResp, SummaryResp};
use hachimi_world_server::web::routes::song::LikeReq;
use hachimi_world_server::web::routes::user::FollowReq;
use uuid::Uuid;

async fn auth_user(env: &mut TestEnvironment) -> TestUser {
    with_new_random_test_user(env).await
}

async fn summary(env: &TestEnvironment) -> SummaryResp {
    env.api.get("/message/summary").await.parse_resp().await.unwrap()
}

async fn mark_read(env: &TestEnvironment, channel: &str) -> MarkReadResp {
    env.api.post("/message/mark_read", &MarkReadReq { channel: channel.into() }).await.parse_resp().await.unwrap()
}

async fn received_likes(env: &TestEnvironment, req: &ReceivedLikesReq) -> ReceivedLikesResp {
    env.api.get_query("/message/received_likes", req).await.parse_resp().await.unwrap()
}

async fn follow(env: &mut TestEnvironment, follower: &TestUser, target_uid: i64) {
    env.api.set_token(follower.token.access_token.clone());
    env.api.post("/user/follow", &FollowReq { target_uid }).await.parse_resp::<serde_json::Value>().await.unwrap();
}

async fn like(env: &mut TestEnvironment, user: &TestUser, song_id: i64) {
    env.api.set_token(user.token.access_token.clone());
    env.api.post("/song/likes/like", &LikeReq { song_id, playback_position_secs: None }).await
        .parse_resp::<serde_json::Value>().await.unwrap();
    // Likes are stored with millisecond-apart times otherwise; keep their order distinct
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
}

#[tokio::test]
async fn test_summary_of_new_user() {
    with_test_environment(|mut env| async move {
        auth_user(&mut env).await;
        let s = summary(&env).await;
        assert_eq!((s.total_unread, s.system_unread, s.like_unread, s.follow_unread), (0, 0, 0, 0));
        // Before the first read, the last 30 days count as unread
        let expected = Utc::now() - Duration::days(30);
        assert!((s.like_read_time - expected).num_seconds().abs() < 60);
        assert!((s.follow_read_time - expected).num_seconds().abs() < 60);
    }).await
}

#[tokio::test]
async fn test_new_followers() {
    with_test_environment(|mut env| async move {
        let b = auth_user(&mut env).await;
        let c = auth_user(&mut env).await;
        let d = auth_user(&mut env).await;
        let a = auth_user(&mut env).await;

        follow(&mut env, &b, a.uid).await;
        follow(&mut env, &c, a.uid).await;
        // A follow from long ago is not new
        sqlx::query("INSERT INTO follows (follower_id, followed_id, create_time) VALUES ($1, $2, $3)")
            .bind(d.uid).bind(a.uid).bind(Utc::now() - Duration::days(40))
            .execute(&env.pool).await.unwrap();

        env.api.set_token(a.token.access_token.clone());
        let s = summary(&env).await;
        assert_eq!((s.follow_unread, s.total_unread), (2, 2));

        let marked = mark_read(&env, "follow").await;
        let s = summary(&env).await;
        assert_eq!(s.follow_unread, 0);
        assert_eq!(s.follow_read_time, marked.read_time);

        // Marking again moves the time forward
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(mark_read(&env, "follow").await.read_time > marked.read_time);

        let e = auth_user(&mut env).await;
        follow(&mut env, &e, a.uid).await;
        env.api.set_token(a.token.access_token.clone());
        assert_eq!(summary(&env).await.follow_unread, 1);
    }).await
}

#[tokio::test]
async fn test_received_likes() {
    with_test_environment(|mut env| async move {
        let contributor = with_test_contributor_user(&mut env).await;
        let b = auth_user(&mut env).await;
        let c = auth_user(&mut env).await;
        let owner = auth_user(&mut env).await;
        let owner_token = owner.token.access_token.clone();
        let contributor_token = contributor.token.access_token.clone();
        let first = create_approved_song(&mut env, &owner_token, &contributor_token, "First").await;
        let second = create_approved_song(&mut env, &owner_token, &contributor_token, "Second").await;

        like(&mut env, &b, first.id).await;
        like(&mut env, &owner, first.id).await; // Own likes don't count
        like(&mut env, &c, first.id).await;
        like(&mut env, &b, second.id).await;

        env.api.set_token(owner_token.clone());
        let s = summary(&env).await;
        assert_eq!(s.like_unread, 3);
        // Plus the two review approval notifications
        assert_eq!((s.system_unread, s.total_unread), (2, 5));

        let page = received_likes(&env, &ReceivedLikesReq { before_time: None, before_song_id: None, limit: None }).await;
        assert!(!page.has_more);
        assert_eq!(page.items.iter().map(|x| x.song_id).collect::<Vec<_>>(), vec![second.id, first.id]);
        let first_item = &page.items[1];
        assert_eq!(first_item.song_display_id, first.jmid);
        assert_eq!(first_item.like_count, 2);
        assert_eq!(first_item.latest_likers.iter().map(|x| x.uid).collect::<Vec<_>>(), vec![c.uid, b.uid]);
        assert_eq!(first_item.latest_likers[0].username, c.name);

        // Paging by the last item's time and song id
        let page1 = received_likes(&env, &ReceivedLikesReq { before_time: None, before_song_id: None, limit: Some(1) }).await;
        assert!(page1.has_more);
        let last = page1.items.last().unwrap();
        let page2 = received_likes(&env, &ReceivedLikesReq {
            before_time: Some(last.latest_like_time),
            before_song_id: Some(last.song_id),
            limit: Some(1),
        }).await;
        assert!(!page2.has_more);
        assert_eq!(page2.items[0].song_id, first.id);

        mark_read(&env, "like").await;
        assert_eq!(summary(&env).await.like_unread, 0);
        // The list itself doesn't change when read
        assert_eq!(received_likes(&env, &ReceivedLikesReq { before_time: None, before_song_id: None, limit: None }).await.items.len(), 2);
    }).await
}

#[tokio::test]
async fn test_system_notifications_count_in_total() {
    with_test_environment(|mut env| async move {
        let user = auth_user(&mut env).await;
        let mut tx = env.pool.begin().await.unwrap();
        send_notification(&mut tx, NewNotification {
            id: Uuid::now_v7(),
            recipient_uid: user.uid,
            notification_type: "test.sample",
            title: "Title".into(),
            body: "Body".into(),
            content_intent: None,
            occurred_at: Utc::now(),
        }).await.unwrap();
        tx.commit().await.unwrap();

        let s = summary(&env).await;
        assert_eq!((s.system_unread, s.total_unread), (1, 1));
    }).await
}

#[tokio::test]
async fn test_invalid_requests() {
    with_test_environment(|mut env| async move {
        auth_user(&mut env).await;
        let err = env.api.post("/message/mark_read", &MarkReadReq { channel: "system".into() }).await
            .parse_resp::<MarkReadResp>().await.unwrap_err();
        assert_eq!(err.code, "invalid_channel");

        let err = env.api.get_query("/message/received_likes", &ReceivedLikesReq {
            before_time: Some(Utc::now()),
            before_song_id: None,
            limit: None,
        }).await.parse_resp::<ReceivedLikesResp>().await.unwrap_err();
        assert_eq!(err.code, "invalid_cursor");

        env.api.clear_token();
        assert_eq!(env.api.get("/message/summary").await.status(), 401);
    }).await
}
