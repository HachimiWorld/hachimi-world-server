mod common;

use crate::common::{assert_is_err, auth, CommonParse, TestEnvironment};
use common::with_test_environment;
use hachimi_world_server::service::user::PublicUserProfile;
use hachimi_world_server::web::routes::user::{
    FollowReq, FollowResp, FollowersListResp, FollowingListReq, FollowingListResp,
    GetProfileReq,
};

/// Helper: follow user and assert success, returning follower_count.
async fn do_follow(env: &mut TestEnvironment, target_uid: i64) -> i64 {
    let resp = env
        .api
        .post("/user/follow", &FollowReq { target_uid })
        .await
        .parse_resp::<FollowResp>()
        .await
        .unwrap();
    resp.follower_count
}

/// Helper: unfollow user and assert success.
async fn do_unfollow(env: &mut TestEnvironment, target_uid: i64) -> i64 {
    let resp = env
        .api
        .post("/user/unfollow", &FollowReq { target_uid })
        .await
        .parse_resp::<FollowResp>()
        .await
        .unwrap();
    resp.follower_count
}

// ─── Follow / Unfollow ─────────────────────────────────────────────────

#[tokio::test]
async fn test_follow_and_unfollow() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;
        let user_b = auth::with_new_random_test_user(&mut env).await;
        env.api.set_token(user_a.token.access_token.clone());

        // Follow
        let count = do_follow(&mut env, user_b.uid).await;
        assert_eq!(count, 1);

        // Check profile of B — should show follower_count = 1
        let profile = env
            .api
            .get_query("/user/profile", &GetProfileReq { uid: user_b.uid })
            .await
            .parse_resp::<PublicUserProfile>()
            .await
            .unwrap();
        assert_eq!(profile.follower_count, 1);
        assert_eq!(profile.following_count, 0);
        assert!(profile.is_following.unwrap());
        assert!(!profile.is_followed_by.unwrap());

        // Check profile of A — should show following_count = 1
        let profile_a = env
            .api
            .get_query("/user/profile", &GetProfileReq { uid: user_a.uid })
            .await
            .parse_resp::<PublicUserProfile>()
            .await
            .unwrap();
        assert_eq!(profile_a.following_count, 1);
        assert_eq!(profile_a.follower_count, 0);

        // Unfollow
        let count = do_unfollow(&mut env, user_b.uid).await;
        assert_eq!(count, 0);

        let profile = env
            .api
            .get_query("/user/profile", &GetProfileReq { uid: user_b.uid })
            .await
            .parse_resp::<PublicUserProfile>()
            .await
            .unwrap();
        assert_eq!(profile.follower_count, 0);
    })
    .await
}

#[tokio::test]
async fn test_follow_is_idempotent() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;
        let user_b = auth::with_new_random_test_user(&mut env).await;
        env.api.set_token(user_a.token.access_token.clone());

        let count1 = do_follow(&mut env, user_b.uid).await;
        let count2 = do_follow(&mut env, user_b.uid).await;
        assert_eq!(count1, 1);
        assert_eq!(count2, 1);
    })
    .await
}

#[tokio::test]
async fn test_unfollow_is_idempotent() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;
        let user_b = auth::with_new_random_test_user(&mut env).await;
        env.api.set_token(user_a.token.access_token.clone());

        do_follow(&mut env, user_b.uid).await;

        do_unfollow(&mut env, user_b.uid).await;
        let count = do_unfollow(&mut env, user_b.uid).await;
        assert_eq!(count, 0);
    })
    .await
}

#[tokio::test]
async fn test_cannot_follow_self() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;

        let resp = env
            .api
            .post("/user/follow", &FollowReq {
                target_uid: user_a.uid,
            })
            .await;
        assert_is_err(resp).await;
    })
    .await
}

#[tokio::test]
async fn test_cannot_unfollow_self() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;

        let resp = env
            .api
            .post("/user/unfollow", &FollowReq {
                target_uid: user_a.uid,
            })
            .await;
        assert_is_err(resp).await;
    })
    .await
}

#[tokio::test]
async fn test_follow_nonexistent_user() {
    with_test_environment(|mut env| async move {
        let _user_a = auth::with_new_random_test_user(&mut env).await;

        let resp = env
            .api
            .post("/user/follow", &FollowReq { target_uid: 99999999 })
            .await;
        assert_is_err(resp).await;
    })
    .await
}

// ─── Profile extensions ────────────────────────────────────────────────

#[tokio::test]
async fn test_profile_shows_mutual_flag() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;
        let user_b = auth::with_new_random_test_user(&mut env).await;
        env.api.set_token(user_a.token.access_token.clone());

        // A follows B
        do_follow(&mut env, user_b.uid).await;

        // A views B's profile — is_following = true, is_followed_by = false
        let profile = env
            .api
            .get_query("/user/profile", &GetProfileReq { uid: user_b.uid })
            .await
            .parse_resp::<PublicUserProfile>()
            .await
            .unwrap();
        assert!(profile.is_following.unwrap());
        assert!(!profile.is_followed_by.unwrap());

        // Now B follows A (switch to user B's token)
        env.api.set_token(user_b.token.access_token.clone());
        do_follow(&mut env, user_a.uid).await;

        // B views A's profile — mutual
        let profile = env
            .api
            .get_query("/user/profile", &GetProfileReq { uid: user_a.uid })
            .await
            .parse_resp::<PublicUserProfile>()
            .await
            .unwrap();
        assert!(profile.is_following.unwrap());
        assert!(profile.is_followed_by.unwrap());
    })
    .await
}

#[tokio::test]
async fn test_profile_follow_flags_none_for_own_profile() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;

        let profile = env
            .api
            .get_query("/user/profile", &GetProfileReq { uid: user_a.uid })
            .await
            .parse_resp::<PublicUserProfile>()
            .await
            .unwrap();
        assert_eq!(profile.is_following, None);
        assert_eq!(profile.is_followed_by, None);
    })
    .await
}

#[tokio::test]
async fn test_profile_follow_flags_none_for_unauthenticated() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;

        // Clear token to simulate unauthenticated request
        env.api.clear_token();

        let profile = env
            .api
            .get_query("/user/profile", &GetProfileReq { uid: user_a.uid })
            .await
            .parse_resp::<PublicUserProfile>()
            .await
            .unwrap();
        assert_eq!(profile.is_following, None);
        assert_eq!(profile.is_followed_by, None);
    })
    .await
}

// ─── Following / Followers Lists ───────────────────────────────────────

#[tokio::test]
async fn test_following_list() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;
        let user_b = auth::with_new_random_test_user(&mut env).await;
        env.api.set_token(user_a.token.access_token.clone());
        let user_c = auth::with_new_random_test_user(&mut env).await;
        env.api.set_token(user_a.token.access_token.clone());

        // A follows B and C
        do_follow(&mut env, user_b.uid).await;
        do_follow(&mut env, user_c.uid).await;

        let resp = env
            .api
            .get_query("/user/following", &FollowingListReq {
                after: None,
                limit: 20,
            })
            .await
            .parse_resp::<FollowingListResp>()
            .await
            .unwrap();

        assert_eq!(resp.items.len(), 2);
        let uids: Vec<i64> = resp.items.iter().map(|i| i.uid).collect();
        assert!(uids.contains(&user_b.uid));
        assert!(uids.contains(&user_c.uid));
    })
    .await
}

#[tokio::test]
async fn test_following_list_mutual_flag() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;
        let user_b = auth::with_new_random_test_user(&mut env).await;
        env.api.set_token(user_a.token.access_token.clone());

        // A follows B
        do_follow(&mut env, user_b.uid).await;

        // Check mutual flag (A's view of following list)
        let resp = env
            .api
            .get_query("/user/following", &FollowingListReq {
                after: None,
                limit: 20,
            })
            .await
            .parse_resp::<FollowingListResp>()
            .await
            .unwrap();
        assert!(!resp.items[0].is_mutual);

        // B follows A back
        env.api.set_token(user_b.token.access_token.clone());
        do_follow(&mut env, user_a.uid).await;

        // B's view of following list — A should be mutual
        let resp = env
            .api
            .get_query("/user/following", &FollowingListReq {
                after: None,
                limit: 20,
            })
            .await
            .parse_resp::<FollowingListResp>()
            .await
            .unwrap();
        assert!(resp.items[0].is_mutual);
    })
    .await
}

#[tokio::test]
async fn test_followers_list() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;
        let user_b = auth::with_new_random_test_user(&mut env).await;
        let user_c = auth::with_new_random_test_user(&mut env).await;

        // B and C follow A
        env.api.set_token(user_b.token.access_token.clone());
        do_follow(&mut env, user_a.uid).await;

        env.api.set_token(user_c.token.access_token.clone());
        do_follow(&mut env, user_a.uid).await;

        // A checks their followers
        env.api.set_token(user_a.token.access_token.clone());
        let resp = env
            .api
            .get_query("/user/followers", &FollowingListReq {
                after: None,
                limit: 20,
            })
            .await
            .parse_resp::<FollowersListResp>()
            .await
            .unwrap();

        assert_eq!(resp.items.len(), 2);
        let uids: Vec<i64> = resp.items.iter().map(|i| i.uid).collect();
        assert!(uids.contains(&user_b.uid));
        assert!(uids.contains(&user_c.uid));
    })
    .await
}

#[tokio::test]
async fn test_following_pagination() {
    with_test_environment(|mut env| async move {
        let user_a = auth::with_new_random_test_user(&mut env).await;
        let user_b = auth::with_new_random_test_user(&mut env).await;
        env.api.set_token(user_a.token.access_token.clone());
        let user_c = auth::with_new_random_test_user(&mut env).await;
        env.api.set_token(user_a.token.access_token.clone());

        do_follow(&mut env, user_b.uid).await;
        do_follow(&mut env, user_c.uid).await;

        // Page 1 with limit=1
        let resp = env
            .api
            .get_query("/user/following", &FollowingListReq {
                after: None,
                limit: 1,
            })
            .await
            .parse_resp::<FollowingListResp>()
            .await
            .unwrap();

        assert_eq!(resp.items.len(), 1);
        assert!(resp.next_cursor.is_some());

        // Page 2
        let resp2 = env
            .api
            .get_query("/user/following", &FollowingListReq {
                after: resp.next_cursor,
                limit: 1,
            })
            .await
            .parse_resp::<FollowingListResp>()
            .await
            .unwrap();

        assert_eq!(resp2.items.len(), 1);
        assert_ne!(resp.items[0].uid, resp2.items[0].uid);
    })
    .await
}

#[tokio::test]
async fn test_following_list_is_empty() {
    with_test_environment(|mut env| async move {
        let _user_a = auth::with_new_random_test_user(&mut env).await;

        let resp = env
            .api
            .get_query("/user/following", &FollowingListReq {
                after: None,
                limit: 20,
            })
            .await
            .parse_resp::<FollowingListResp>()
            .await
            .unwrap();

        assert!(resp.items.is_empty());
        assert!(resp.next_cursor.is_none());
    })
    .await
}
