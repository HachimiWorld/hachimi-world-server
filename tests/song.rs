mod common;

use crate::common::auth::{with_new_random_test_user, with_test_contributor_user};
use crate::common::publish::create_approved_song;
use crate::common::song::create_tags;
use crate::common::{assert_is_err, assert_is_ok, CommonParse};
use crate::common::{with_test_environment, TestEnvironment};
use hachimi_world_server::web::routes::song::{
    DetailReq,
    DetailResp,
    LikeReq,
    LikeStatusResp,
    MyLikesReq,
    MyLikesResp,
    PageByUserReq,
    RecentReq,
    RecentResp,
    SearchReq,
    SearchResp,
    TagSearchReq,
    TagSearchResp,
};

/// Helper: creates a user, a contributor, and an approved song. Returns the song and both tokens.
async fn setup_song(env: &mut TestEnvironment) -> (
    common::auth::TestUser,
    common::auth::TestUser,
    common::publish::ApprovedSong,
) {
    let owner = with_new_random_test_user(env).await;
    let contributor = with_test_contributor_user(env).await;

    let owner_token = owner.token.access_token.clone();
    let contributor_token = contributor.token.access_token.clone();

    let song = create_approved_song(
        env,
        &owner_token,
        &contributor_token,
        "Test Song",
    ).await;

    env.api.set_token(owner_token);
    (owner, contributor, song)
}

#[tokio::test]
async fn test_get_song_detail() {
    with_test_environment(|mut env| async move {
        let (_owner, _contributor, song) = setup_song(&mut env).await;

        // Fetch detail by JMID
        let resp: DetailResp = env.api
            .get_query("/song/detail", &DetailReq { id: song.jmid.clone() })
            .await.parse_resp().await.unwrap();

        assert_eq!(resp.id, song.id);
        assert!(resp.title.starts_with("Test Song-"));
        assert!(resp.subtitle.starts_with("subtitle-"));
    }).await;
}

#[tokio::test]
async fn test_get_recent_songs() {
    with_test_environment(|mut env| async move {
        let owner = with_new_random_test_user(&mut env).await;
        let contributor = with_test_contributor_user(&mut env).await;
        let owner_token = owner.token.access_token.clone();
        let contributor_token = contributor.token.access_token.clone();

        // Create 5 approved songs
        let mut song_ids = Vec::new();
        for i in 0..5 {
            let song = create_approved_song(
                &mut env,
                &owner_token,
                &contributor_token,
                &format!("Recent Song {i}"),
            ).await;
            song_ids.push(song.id);
        }

        // Fetch recent songs (no params = default pagination)
        let resp: RecentResp = env.api
            .get_query("/song/recent_v2", &RecentReq { cursor: None, limit: None, after: None })
            .await.parse_resp().await.unwrap();

        assert!(!resp.songs.is_empty(), "Should have recent songs");

        // Verify at least one of our songs appears (newest first, so song 4 should be first)
        let recent_ids: Vec<i64> = resp.songs.iter().map(|s| s.id).collect();
        assert!(recent_ids.iter().any(|id| song_ids.contains(id)),
            "Created songs should appear in recent");

        // Test cursor-based pagination with limit=2
        let page1: RecentResp = env.api
            .get_query("/song/recent_v2", &RecentReq { cursor: None, limit: Some(2), after: None })
            .await.parse_resp().await.unwrap();
        assert_eq!(page1.songs.len(), 2, "First page should have 2 songs");

        if let Some(last) = page1.songs.last() {
            let page2: RecentResp = env.api
                .get_query("/song/recent_v2", &RecentReq {
                    cursor: Some(last.create_time),
                    limit: Some(2),
                    after: None,
                })
                .await.parse_resp().await.unwrap();

            // No overlap between pages
            let page2_ids: std::collections::HashSet<i64> = page2.songs.iter().map(|s| s.id).collect();
            assert!(page1.songs.iter().all(|s| !page2_ids.contains(&s.id)),
                "Pages should not overlap");
        }
    }).await;
}

#[tokio::test]
async fn test_get_recommend_songs() {
    with_test_environment(|mut env| async move {
        let (_owner, _contributor, _song) = setup_song(&mut env).await;
        create_tags(&env.api).await;

        let resp: RecentResp = env.api.get("/song/recommend")
            .await.parse_resp().await.unwrap();
        // Recommend may return empty if the algorithm doesn't pick our songs,
        // but with tags created it should have candidates.
        // Just verify the endpoint returns a valid response (no panic/error).
        let _ = resp;
    }).await;
}

#[tokio::test]
async fn test_get_weekly_hot_songs() {
    with_test_environment(|mut env| async move {
        let (_owner, _contributor, _song) = setup_song(&mut env).await;

        let resp: RecentResp = env.api.get("/song/hot/weekly")
            .await.parse_resp().await.unwrap();
        // Just verify the endpoint returns a valid response (no panic/error).
        let _ = resp;
    }).await;
}

#[tokio::test]
async fn test_search() {
    with_test_environment(|mut env| async move {
        let owner = with_new_random_test_user(&mut env).await;
        let contributor = with_test_contributor_user(&mut env).await;
        let owner_token = owner.token.access_token.clone();
        let contributor_token = contributor.token.access_token.clone();

        // Create a song with a distinctive searchable word in the title
        let unique_word = format!("ZQX{}", uuid::Uuid::new_v4().simple().to_string().replace('-', ""));
        let song_title = format!("{unique_word}Song");
        let song = create_approved_song(
            &mut env,
            &owner_token,
            &contributor_token,
            &song_title,
        ).await;

        // Poll until the song is searchable (Meilisearch indexing is async)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let search_result: SearchResp = env.api.get_query("/song/search", &SearchReq {
                q: unique_word.clone(),
                limit: Some(10),
                offset: Some(0),
                filter: None,
                sort_by: None,
            }).await.parse_resp().await.unwrap();

            if search_result.hits.iter().any(|h| h.id == song.id) {
                return; // Success
            }

            assert!(
                std::time::Instant::now() < deadline,
                "Song {} ('{song_title}') was not searchable for query '{unique_word}' within timeout",
                song.id
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }).await;
}

#[tokio::test]
async fn test_create_and_search_tags() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;
        create_tags(&env.api).await;

        // Search for a partial tag name
        let resp: TagSearchResp = env.api
            .get_query("/song/tag/search", &TagSearchReq { query: "原教".to_string() })
            .await.parse_resp().await.unwrap();

        let first_tag = resp.result.first().unwrap();
        assert_eq!("原教旨", first_tag.name);
        assert_eq!(None, first_tag.description);
    }).await;
}

#[tokio::test]
async fn test_page_by_users() {
    with_test_environment(|mut env| async move {
        let owner = with_new_random_test_user(&mut env).await;
        let contributor = with_test_contributor_user(&mut env).await;
        let owner_token = owner.token.access_token.clone();
        let contributor_token = contributor.token.access_token.clone();

        // Create 3 songs as the owner
        let mut song_ids = Vec::new();
        for i in 0..3 {
            let song = create_approved_song(
                &mut env,
                &owner_token,
                &contributor_token,
                &format!("User Song {i}"),
            ).await;
            song_ids.push(song.id);
        }
        env.api.set_token(owner_token);

        // Test first page (page=0, size=2)
        let resp: RecentResp = env.api.get_query("/song/page_by_user", &PageByUserReq {
            user_id: owner.uid,
            page: Some(0),
            size: Some(2),
        }).await.parse_resp().await.unwrap();

        assert_eq!(resp.songs.len(), 2, "First page should have 2 songs");
        assert!(resp.songs.iter().all(|s| s.uploader_uid == owner.uid),
            "All songs should belong to the owner");

        // Test second page (page=1, size=2)
        let resp2: RecentResp = env.api.get_query("/song/page_by_user", &PageByUserReq {
            user_id: owner.uid,
            page: Some(1),
            size: Some(2),
        }).await.parse_resp().await.unwrap();

        assert!(!resp2.songs.is_empty(), "Second page should have remaining songs");

        // Assert no overlap between pages
        let resp2_ids: std::collections::HashSet<i64> = resp2.songs.iter().map(|s| s.id).collect();
        assert!(resp.songs.iter().all(|s| !resp2_ids.contains(&s.id)),
            "Pages should not overlap");
    }).await;
}

#[tokio::test]
async fn test_likes() {
    with_test_environment(|mut env| async move {
        let (_owner, _contributor, song) = setup_song(&mut env).await;

        let like_req = LikeReq {
            song_id: song.id,
            playback_position_secs: Some(123),
        };

        // Like
        assert_is_ok(env.api.post("/song/likes/like", &like_req).await).await;

        // Check liked status
        let status: LikeStatusResp = env.api
            .get_query("/song/likes/status", &like_req).await
            .parse_resp().await.unwrap();
        assert!(status.liked, "song should be liked after /like");

        // Check my likes page
        let page: MyLikesResp = env.api
            .get_query("/song/likes/page_my_likes", &MyLikesReq { page_index: 0, page_size: 10 })
            .await.parse_resp().await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.data.len(), 1);
        assert_eq!(page.data[0].song_data.id, song.id);

        // Unlike
        assert_is_ok(env.api.post("/song/likes/unlike", &like_req).await).await;

        // Check unliked status
        let status: LikeStatusResp = env.api
            .get_query("/song/likes/status", &like_req).await
            .parse_resp().await.unwrap();
        assert!(!status.liked, "song should be unliked after /unlike");

        // Check my likes page is empty
        let page: MyLikesResp = env.api
            .get_query("/song/likes/page_my_likes", &MyLikesReq { page_index: 0, page_size: 10 })
            .await.parse_resp().await.unwrap();
        assert_eq!(page.total, 0);
        assert!(page.data.is_empty());
    }).await;
}

#[tokio::test]
async fn test_likes_validation() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        let invalid_index = env.api
            .get_query("/song/likes/page_my_likes", &MyLikesReq { page_index: -1, page_size: 10 })
            .await;
        assert_is_err(invalid_index).await;

        let invalid_size = env.api
            .get_query("/song/likes/page_my_likes", &MyLikesReq { page_index: 0, page_size: 0 })
            .await;
        assert_is_err(invalid_size).await;
    }).await;
}