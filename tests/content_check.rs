mod common;

use crate::common::typesafe::TEST_MODEL;
use crate::common::{auth, with_test_environment, with_test_environment_with, ApiClient, CommonParse, TestOptions};
use hachimi_world_server::db::content_check::{ContentCheck, ContentCheckDao, ContentPolicyDao};
use hachimi_world_server::db::user::{User, UserDao};
use hachimi_world_server::db::CrudDao;
use hachimi_world_server::web::routes::user::UpdateProfileReq;
use sqlx::PgPool;

const CHECKS_ON: TestOptions = TestOptions { content_check: true };

async fn update(api: &ApiClient, username: &str, bio: Option<&str>) -> Result<(), String> {
    api.post("/user/update_profile", &UpdateProfileReq {
        username: username.to_string(),
        bio: bio.map(str::to_string),
        gender: None,
    }).await.parse_resp::<()>().await.map_err(|e| e.code)
}

/// Read from the database: `/user/profile` is cached and `update_profile` doesn't clear it.
async fn profile(pool: &PgPool, uid: i64) -> User {
    UserDao::get_by_id(pool, uid).await.unwrap().unwrap()
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

        let profile = profile(&env.pool, user.uid).await;
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
        let before = profile(&env.pool, user.uid).await;

        assert_eq!(update(&env.api, "坏名#sexual", Some("喜欢听歌")).await, Err("username_rejected".to_string()));

        // Nothing is saved, not even the bio that passed
        let after = profile(&env.pool, user.uid).await;
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
        let before = profile(&env.pool, user.uid).await;

        assert_eq!(update(&env.api, "哈基米", Some("加群领福利 #spam")).await, Err("bio_rejected".to_string()));

        let after = profile(&env.pool, user.uid).await;
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
        assert_eq!(profile(&env.pool, user.uid).await.username, "名字#down");

        // The answers don't match the policy
        update(&env.api, "名字#odd", None).await.unwrap();
        assert_eq!(profile(&env.pool, user.uid).await.username, "名字#odd");

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

        assert_eq!(profile(&env.pool, user.uid).await.username, "坏名#sexual");
        assert!(checks(&env.pool, user.uid).await.is_empty());
    }).await
}
