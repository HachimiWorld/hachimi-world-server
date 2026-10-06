mod common;

use crate::common::{auth, CommonParse, TestEnvironment};
use chrono::{Duration, Utc};
use common::with_test_environment;
use hachimi_world_server::db::notification::NotificationDao;
use hachimi_world_server::service::notification::{
    send_notification, visible_since, ContentIntent, NewNotification, SendError, SendOutcome, RETENTION_DAYS,
};
use hachimi_world_server::web::routes::notification::{
    DetailReq, ListReq, ListResp, MarkAllReadResp, MarkReadReq, MarkReadResp, NotificationItem,
    UnreadCountResp,
};
use serde_json::{json, Map};
use uuid::Uuid;

fn new_notification(recipient_uid: i64, title: &str) -> NewNotification {
    NewNotification {
        id: Uuid::now_v7(),
        recipient_uid,
        notification_type: "test.sample",
        title: title.to_string(),
        body: format!("{title} body"),
        content_intent: None,
        occurred_at: Utc::now(),
    }
}

async fn send(env: &TestEnvironment, n: NewNotification) -> Result<SendOutcome, SendError> {
    let mut tx = env.pool.begin().await.unwrap();
    let outcome = send_notification(&mut tx, n).await?;
    tx.commit().await.unwrap();
    Ok(outcome)
}

/// Sends a notification titled `title` and returns its id.
async fn send_titled(env: &TestEnvironment, recipient_uid: i64, title: &str) -> Uuid {
    let n = new_notification(recipient_uid, title);
    let id = n.id;
    assert_eq!(send(env, n).await.unwrap(), SendOutcome::Created);
    id
}

async fn list(env: &TestEnvironment, before_id: Option<Uuid>, limit: Option<i64>) -> ListResp {
    env.api.get_query("/notification/list", &ListReq {
        before_id: before_id.map(|x| x.to_string()),
        limit,
    }).await.parse_resp::<ListResp>().await.unwrap()
}

async fn unread_count(env: &TestEnvironment) -> i64 {
    env.api.get("/notification/unread_count").await
        .parse_resp::<UnreadCountResp>().await.unwrap()
        .unread_count
}

async fn mark_read(env: &TestEnvironment, id: &str) -> Result<MarkReadResp, String> {
    env.api.post("/notification/mark_read", &MarkReadReq { notification_id: id.to_string() }).await
        .parse_resp::<MarkReadResp>().await
        .map_err(|e| e.code)
}

async fn detail(env: &TestEnvironment, id: &str) -> Result<NotificationItem, String> {
    env.api.get_query("/notification/detail", &DetailReq { notification_id: id.to_string() }).await
        .parse_resp::<NotificationItem>().await
        .map_err(|e| e.code)
}

#[tokio::test]
async fn test_list_and_unread_count() {
    with_test_environment(|mut env| async move {
        let user_b = auth::with_new_random_test_user(&mut env).await;
        let user_a = auth::with_new_random_test_user(&mut env).await;

        let first = send_titled(&env, user_a.uid, "First").await;
        let second = send_titled(&env, user_a.uid, "Second").await;
        send_titled(&env, user_b.uid, "For B").await;

        let resp = list(&env, None, None).await;
        assert!(!resp.has_more);
        assert_eq!(resp.items.iter().map(|x| x.notification_id).collect::<Vec<_>>(), vec![second, first]);
        let item = &resp.items[0];
        assert_eq!(item.notification_type, "test.sample");
        assert_eq!(item.title, "Second");
        assert_eq!(item.body, "Second body");
        assert!(item.read_time.is_none());
        assert!(item.content_intent.is_none());
        assert_eq!(unread_count(&env).await, 2);

        env.api.set_token(user_b.token.access_token.clone());
        let resp = list(&env, None, None).await;
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].title, "For B");

        // Empty inbox
        auth::with_new_random_test_user(&mut env).await;
        let resp = list(&env, None, None).await;
        assert!(resp.items.is_empty());
        assert!(!resp.has_more);
        assert_eq!(unread_count(&env).await, 0);
    }).await
}

#[tokio::test]
async fn test_pagination() {
    with_test_environment(|mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;
        let mut ids = Vec::new();
        for i in 0..5 {
            ids.push(send_titled(&env, user.uid, &format!("N{i}")).await);
        }
        ids.reverse();

        let mut seen = Vec::new();
        let mut before_id = None;
        loop {
            let resp = list(&env, before_id, Some(2)).await;
            assert!(resp.items.len() <= 2);
            seen.extend(resp.items.iter().map(|x| x.notification_id));
            if !resp.has_more {
                break;
            }
            before_id = resp.items.last().map(|x| x.notification_id);
        }
        assert_eq!(seen, ids);

        // Limit is clamped
        assert_eq!(list(&env, None, Some(0)).await.items.len(), 1);
        assert_eq!(list(&env, None, Some(1000)).await.items.len(), 5);

        let err = env.api.get_query("/notification/list", &ListReq { before_id: Some("abc".to_string()), limit: None }).await
            .parse_resp::<ListResp>().await.unwrap_err();
        assert_eq!(err.code, "invalid_notification_id");
    }).await
}

#[tokio::test]
async fn test_detail() {
    with_test_environment(|mut env| async move {
        let other = auth::with_new_random_test_user(&mut env).await;
        let user = auth::with_new_random_test_user(&mut env).await;
        let mine = send_titled(&env, user.uid, "Mine").await;
        let theirs = send_titled(&env, other.uid, "Theirs").await;

        let item = detail(&env, &mine.to_string()).await.unwrap();
        assert_eq!(item.notification_id, mine);
        assert_eq!(item.title, "Mine");
        // Reading the detail doesn't mark it read
        assert!(item.read_time.is_none());
        assert_eq!(unread_count(&env).await, 1);

        assert_eq!(detail(&env, &theirs.to_string()).await.unwrap_err(), "notification_unavailable");
        assert_eq!(detail(&env, &Uuid::now_v7().to_string()).await.unwrap_err(), "notification_unavailable");
        assert_eq!(detail(&env, "not-a-uuid").await.unwrap_err(), "invalid_notification_id");
    }).await
}

#[tokio::test]
async fn test_mark_read() {
    with_test_environment(|mut env| async move {
        let other = auth::with_new_random_test_user(&mut env).await;
        let user = auth::with_new_random_test_user(&mut env).await;
        let a = send_titled(&env, user.uid, "A").await;
        send_titled(&env, user.uid, "B").await;
        let theirs = send_titled(&env, other.uid, "Theirs").await;

        let resp = mark_read(&env, &a.to_string()).await.unwrap();
        assert_eq!(resp.notification_id, a);
        assert_eq!(resp.unread_count, 1);
        assert_eq!(detail(&env, &a.to_string()).await.unwrap().read_time, Some(resp.read_time));

        // Marking again keeps the first read time
        let again = mark_read(&env, &a.to_string()).await.unwrap();
        assert_eq!(again.read_time, resp.read_time);
        assert_eq!(again.unread_count, 1);

        assert_eq!(mark_read(&env, &theirs.to_string()).await.unwrap_err(), "notification_unavailable");
        assert_eq!(mark_read(&env, "not-a-uuid").await.unwrap_err(), "invalid_notification_id");

        env.api.set_token(other.token.access_token.clone());
        assert_eq!(unread_count(&env).await, 1);
    }).await
}

#[tokio::test]
async fn test_mark_all_read() {
    with_test_environment(|mut env| async move {
        let other = auth::with_new_random_test_user(&mut env).await;
        let user = auth::with_new_random_test_user(&mut env).await;
        for i in 0..3 {
            send_titled(&env, user.uid, &format!("N{i}")).await;
        }
        send_titled(&env, other.uid, "Theirs").await;

        // Covers notifications the client hasn't loaded
        let resp = env.api.post("/notification/mark_all_read", &json!({})).await
            .parse_resp::<MarkAllReadResp>().await.unwrap();
        assert_eq!(resp.marked_count, 3);
        assert_eq!(resp.unread_count, 0);
        assert!(list(&env, None, None).await.items.iter().all(|x| x.read_time.is_some()));

        let resp = env.api.post("/notification/mark_all_read", &json!({})).await
            .parse_resp::<MarkAllReadResp>().await.unwrap();
        assert_eq!(resp.marked_count, 0);

        env.api.set_token(other.token.access_token.clone());
        assert_eq!(unread_count(&env).await, 1);
    }).await
}

#[tokio::test]
async fn test_retention() {
    with_test_environment(|mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;
        let fresh = send_titled(&env, user.uid, "Fresh").await;
        let mut old = new_notification(user.uid, "Old");
        old.occurred_at = Utc::now() - Duration::days(RETENTION_DAYS + 1);
        let old_id = old.id;
        send(&env, old).await.unwrap();

        let items = list(&env, None, None).await.items;
        assert_eq!(items.iter().map(|x| x.notification_id).collect::<Vec<_>>(), vec![fresh]);
        assert_eq!(unread_count(&env).await, 1);
        assert_eq!(detail(&env, &old_id.to_string()).await.unwrap_err(), "notification_unavailable");
        assert_eq!(mark_read(&env, &old_id.to_string()).await.unwrap_err(), "notification_unavailable");

        let deleted = NotificationDao::delete_created_before(&env.pool, visible_since(), 1000).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(NotificationDao::get_by_id(&env.pool, old_id).await.unwrap().is_none());
        assert!(NotificationDao::get_by_id(&env.pool, fresh).await.unwrap().is_some());
    }).await
}

#[tokio::test]
async fn test_send_idempotency() {
    with_test_environment(|mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        let n = new_notification(user.uid, "Once");
        let id = n.id;
        assert_eq!(send(&env, n).await.unwrap(), SendOutcome::Created);

        // Retrying with the same id and content
        let mut retry = new_notification(user.uid, "Once");
        retry.id = id;
        assert_eq!(send(&env, retry).await.unwrap(), SendOutcome::AlreadyExists);
        assert_eq!(list(&env, None, None).await.items.len(), 1);

        // Reusing the id for different content
        let mut conflict = new_notification(user.uid, "Different");
        conflict.id = id;
        assert!(matches!(send(&env, conflict).await, Err(SendError::IdempotencyConflict(x)) if x == id));

        // Rolled back with the business transaction
        let n = new_notification(user.uid, "Rolled back");
        let rolled_back = n.id;
        let mut tx = env.pool.begin().await.unwrap();
        send_notification(&mut tx, n).await.unwrap();
        tx.rollback().await.unwrap();
        assert!(NotificationDao::get_by_id(&env.pool, rolled_back).await.unwrap().is_none());
        assert_eq!(unread_count(&env).await, 1);
    }).await
}

#[tokio::test]
async fn test_send_validation() {
    with_test_environment(|mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;
        let uid = user.uid;
        let env = &env;
        let invalid = |n: NewNotification| async move { matches!(send(env, n).await, Err(SendError::Invalid(_))) };

        let mut n = new_notification(uid, "x");
        n.id = Uuid::new_v4();
        assert!(invalid(n).await);

        for bad_type in ["test", "Test.sample", "test.", ".sample", "test.sam ple", "test.1sample"] {
            let mut n = new_notification(uid, "x");
            n.notification_type = bad_type;
            assert!(invalid(n).await, "{bad_type}");
        }

        let mut n = new_notification(uid, "x");
        n.title = String::new();
        assert!(invalid(n).await);

        let mut n = new_notification(uid, "x");
        n.title = "a".repeat(121);
        assert!(invalid(n).await);

        let mut n = new_notification(uid, "x");
        n.title = "line\nbreak".to_string();
        assert!(invalid(n).await);

        let mut n = new_notification(uid, "x");
        n.body = "tab\there".to_string();
        assert!(invalid(n).await);

        let mut n = new_notification(uid, "x");
        n.body = "line\nbreak is fine in body".to_string();
        assert_eq!(send(env, n).await.unwrap(), SendOutcome::Created);

        let mut n = new_notification(uid, "x");
        n.content_intent = Some(ContentIntent::new("view", Map::new()));
        assert!(invalid(n).await);

        let mut data = Map::new();
        data.insert("padding".to_string(), json!("a".repeat(1100)));
        let mut n = new_notification(uid, "x");
        n.content_intent = Some(ContentIntent::new("profile.edit", data));
        assert!(invalid(n).await);
    }).await
}

#[tokio::test]
async fn test_content_intent() {
    with_test_environment(|mut env| async move {
        let user = auth::with_new_random_test_user(&mut env).await;

        let mut data = Map::new();
        data.insert("song_id".to_string(), json!(1));
        data.insert("comment_id".to_string(), json!(2));
        let mut n = new_notification(user.uid, "With intent");
        n.content_intent = Some(ContentIntent::new("song.comment.view", data.clone()));
        let id = n.id;
        send(&env, n).await.unwrap();

        let item = detail(&env, &id.to_string()).await.unwrap();
        assert_eq!(item.content_intent, Some(ContentIntent::new("song.comment.view", data)));

        // A malformed stored intent is returned as null, and the text is still shown
        sqlx::query("UPDATE notifications SET content_intent = '{\"action\": 1}'::jsonb WHERE id = $1")
            .bind(id)
            .execute(&env.pool).await.unwrap();
        let item = detail(&env, &id.to_string()).await.unwrap();
        assert!(item.content_intent.is_none());
        assert_eq!(item.title, "With intent");
    }).await
}

#[tokio::test]
async fn test_requires_login() {
    with_test_environment(|mut env| async move {
        env.api.clear_token();
        assert_eq!(env.api.get("/notification/unread_count").await.status(), 401);
        assert_eq!(env.api.get("/notification/list").await.status(), 401);
        assert_eq!(env.api.post("/notification/mark_all_read", &json!({})).await.status(), 401);
    }).await
}
