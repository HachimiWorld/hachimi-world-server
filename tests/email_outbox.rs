mod common;

use crate::common::auth::{with_new_random_test_user, with_test_contributor_user};
use crate::common::publish::publish_template;
use crate::common::{assert_is_ok, CommonParse, TestEnvironment};
use chrono::{Duration, Utc};
use common::with_test_environment;
use hachimi_world_server::db::email_outbox::{EmailOutbox, EmailOutboxDao};
use hachimi_world_server::service::email_outbox;
use hachimi_world_server::service::mailer::{EmailConfig, NotificationEmail};
use hachimi_world_server::util::redlock::RedLock;
use hachimi_world_server::web::routes::publish::review::{ApproveReviewReq, ReviewCommentCreateReq};
use hachimi_world_server::web::routes::publish::PublishResp;
use uuid::Uuid;

const MAINTAINER_EMAIL: &str = "maintainer@example.com";

fn disabled_email() -> EmailConfig {
    EmailConfig {
        disabled: true,
        host: "email.example.com".into(),
        username: "noreply@example.com".into(),
        password: "12345678".into(),
        no_reply_email: "noreply@example.com".into(),
    }
}

/// Fails right away: nothing listens for SMTP on this host.
fn unreachable_email() -> EmailConfig {
    EmailConfig { disabled: false, host: "127.0.0.1".into(), ..disabled_email() }
}

async fn all_emails(env: &TestEnvironment) -> Vec<EmailOutbox> {
    sqlx::query_as::<_, EmailOutbox>("SELECT * FROM email_outbox ORDER BY id").fetch_all(&env.pool).await.unwrap()
}

async fn enqueue_committed(env: &TestEnvironment, to: &str) -> Uuid {
    let mut tx = env.pool.begin().await.unwrap();
    let id = email_outbox::enqueue(&mut tx, to, NotificationEmail { subject: "Subject".into(), body: "Body".into() }).await.unwrap();
    tx.commit().await.unwrap();
    id
}

#[tokio::test]
async fn test_review_flow_enqueues_emails_and_relay_sends_them() {
    with_test_environment(|mut env| async move {
        let uploader = with_new_random_test_user(&mut env).await;
        let contributor = with_test_contributor_user(&mut env).await;

        env.api.set_token(uploader.token.access_token.clone());
        let mut req = publish_template(&env).await;
        req.title = "Outbox Song".into();
        let published: PublishResp = env.api.post("/publish/publish", &req).await.parse_resp().await.unwrap();

        // The uploader comments: only the contributor is emailed
        assert_is_ok(env.api.post("/publish/review/comment/create", &ReviewCommentCreateReq {
            review_id: published.review_id,
            content: "Please take a look".into(),
        }).await).await;

        env.api.set_token(contributor.token.access_token.clone());
        assert_is_ok(env.api.post("/publish/review/approve", &ApproveReviewReq {
            review_id: published.review_id,
            comment: Some("Nice".into()),
        }).await).await;

        let emails = all_emails(&env).await;
        let summary: Vec<(&str, &str)> = emails.iter().map(|x| (x.to_address.as_str(), x.subject.as_str())).collect();
        let comment_subject = format!("稿件评论更新：{}", published.song_display_id);
        assert_eq!(summary, vec![
            (MAINTAINER_EMAIL, "有新的稿件待审核"),
            (MAINTAINER_EMAIL, comment_subject.as_str()),
            (uploader.email.as_str(), "您提交的作品已通过审核"),
        ]);
        assert!(emails[2].body.contains("《Outbox Song》") && emails[2].body.ends_with("审核留言：Nice"));
        assert!(emails.iter().all(|x| x.sent_time.is_none() && x.attempt_count == 0));

        let red_lock = RedLock::new(env.redis.clone()).unwrap();
        let attempted = email_outbox::relay(&env.pool, &red_lock, &disabled_email()).await.unwrap();
        assert_eq!(attempted, 3);
        assert!(all_emails(&env).await.iter().all(|x| x.sent_time.is_some() && x.attempt_count == 1 && x.last_error.is_none()));

        // Nothing is due anymore
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(email_outbox::relay(&env.pool, &red_lock, &disabled_email()).await.unwrap(), 0);
    }).await
}

#[tokio::test]
async fn test_rolled_back_email_is_never_sent() {
    with_test_environment(|env| async move {
        let mut tx = env.pool.begin().await.unwrap();
        email_outbox::enqueue(&mut tx, "a@example.com", NotificationEmail { subject: "S".into(), body: "B".into() }).await.unwrap();
        tx.rollback().await.unwrap();
        assert!(all_emails(&env).await.is_empty());
    }).await
}

#[tokio::test]
async fn test_failed_email_is_retried_then_given_up() {
    with_test_environment(|env| async move {
        let red_lock = RedLock::new(env.redis.clone()).unwrap();
        let id = enqueue_committed(&env, "a@example.com").await;

        assert_eq!(email_outbox::relay(&env.pool, &red_lock, &unreachable_email()).await.unwrap(), 1);
        let email = EmailOutboxDao::get_by_id(&env.pool, id).await.unwrap().unwrap();
        assert_eq!(email.attempt_count, 1);
        assert!(email.sent_time.is_none() && email.dead_time.is_none());
        assert!(email.last_error.is_some());
        assert!(email.available_time > Utc::now() + Duration::seconds(30), "retried after a delay");

        // Not due yet, so another run leaves it alone
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(email_outbox::relay(&env.pool, &red_lock, &unreachable_email()).await.unwrap(), 0);

        // The last allowed attempt fails too
        let mut last = email.clone();
        last.attempt_count = 7;
        last.available_time = Utc::now() - Duration::seconds(1);
        EmailOutboxDao::update_delivery(&env.pool, &last).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(email_outbox::relay(&env.pool, &red_lock, &unreachable_email()).await.unwrap(), 1);
        let email = EmailOutboxDao::get_by_id(&env.pool, id).await.unwrap().unwrap();
        assert_eq!(email.attempt_count, 8);
        assert!(email.dead_time.is_some() && email.sent_time.is_none());

        // Dead emails are not retried
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(email_outbox::relay(&env.pool, &red_lock, &disabled_email()).await.unwrap(), 0);
    }).await
}

#[tokio::test]
async fn test_relay_skips_while_another_replica_holds_the_lock() {
    with_test_environment(|env| async move {
        let red_lock = RedLock::new(env.redis.clone()).unwrap();
        enqueue_committed(&env, "a@example.com").await;

        let _held = red_lock.try_lock("email_outbox:relay").await.unwrap().unwrap();
        assert_eq!(email_outbox::relay(&env.pool, &red_lock, &disabled_email()).await.unwrap(), 0);
        assert!(all_emails(&env).await[0].sent_time.is_none());
    }).await
}

#[tokio::test]
async fn test_cleanup_deletes_old_sent_and_dead_emails() {
    with_test_environment(|env| async move {
        let now = Utc::now();
        let mut ids = Vec::new();
        for (sent_days_ago, dead_days_ago) in [(Some(31), None), (Some(29), None), (None, Some(91)), (None, Some(89)), (None, None)] {
            let id = enqueue_committed(&env, "a@example.com").await;
            let mut email = EmailOutboxDao::get_by_id(&env.pool, id).await.unwrap().unwrap();
            email.sent_time = sent_days_ago.map(|x| now - Duration::days(x));
            email.dead_time = dead_days_ago.map(|x| now - Duration::days(x));
            EmailOutboxDao::update_delivery(&env.pool, &email).await.unwrap();
            ids.push(id);
        }

        assert_eq!(email_outbox::cleanup(&env.pool).await.unwrap(), 2);
        let left: Vec<Uuid> = all_emails(&env).await.into_iter().map(|x| x.id).collect();
        assert_eq!(left, vec![ids[1], ids[3], ids[4]]);
    }).await
}
