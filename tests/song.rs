mod common;

use crate::common::auth::with_new_random_test_user;
use crate::common::{assert_is_err, assert_is_ok, CommonParse};
use crate::common::{with_test_environment, TestEnvironment};
use futures::future::join_all;
use hachimi_world_server::service::song_like;
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
    TagCreateReq,
    TagSearchReq,
    TagSearchResp,
};
use std::time::SystemTime;

#[tokio::test]
async fn test_get_likes() {
    with_test_environment(|mut env| async move {
        // Get likes
        let resp: DetailResp = env.api.get_query("/song/detail", &DetailReq { id: "JM-IOEW-474".to_string() }).await.parse_resp().await.unwrap();
        println!("{:#?}", resp);
    }).await;
}

#[tokio::test]
async fn test_get_recent_songs() {
    with_test_environment(|mut env| async move {
        // Compatible check for API before 251102
        let resp: RecentResp = env.api.get("/song/recent_v2").await.parse_resp().await.unwrap();
        println!("recent: {:?}", resp.songs);

        // Pagination test
        let resp: RecentResp = env.api.get_query("/song/recent_v2", &RecentReq {
            cursor: None,
            limit: None,
            after: None,
        }).await.parse_resp().await.unwrap();
        println!("recent2: {:?}", resp);

        let last = resp.songs.last().unwrap();
        let next_page: RecentResp = env.api.get_query("/song/recent_v2", &RecentReq {
            cursor: Some(last.create_time),
            limit: None,
            after: None,
        }).await.parse_resp().await.unwrap();
        
        // Check resp.songs and next_page.songs if not duplicate
        assert!(resp.songs.iter().all(|song1|
            !next_page.songs.iter().any(|song2| song1.id == song2.id)
        ));
    }).await;
}

#[tokio::test]
async fn test_get_recommend_songs() {
    with_test_environment(|mut env| async move {
        let test = with_new_random_test_user(&mut env).await;
        let resp: RecentResp = env.api.get("/song/recommend").await.parse_resp().await.unwrap();
        println!("recent: {:?}", resp.songs);
    }).await;
}

#[tokio::test]
async fn test_get_weekly_hot_songs() {
    with_test_environment(|mut env| async move {
        let resp: RecentResp = env.api.get("/song/hot/weekly").await.parse_resp().await.unwrap();
        println!("recent: {:?}", resp.songs);
    }).await;
}

#[tokio::test]
async fn test_search() {
    with_test_environment(|mut env| async move {
        // TODO: Add test fixtures
        // Test search
        let search_result: SearchResp = env.api.get_query("/song/search", &SearchReq {
            q: "基米".to_string(),
            limit: None,
            offset: None,
            filter: None,
            sort_by: None,
        }).await.parse_resp().await.unwrap();
        println!("{:#?}", search_result);
    }).await
}

#[tokio::test]
async fn test_create_and_search_tags() {
    with_test_environment(|mut env| async move {
        // TODO[test](song): We should add cleanup code to make the test repeatable.
        let tags = vec!["原教旨", "流行", "古典", "人声翻唱", "摇滚", "R&B", "民谣"];
        for x in tags {
            let resp = env.api.post(
                "/song/tag/create",
                &TagCreateReq {
                    name: x.to_string(),
                    description: None,
                },
            ).await;
            assert_is_ok(resp).await;
        }

        // Get tag
        let resp: TagSearchResp = env
            .api
            .get_query(
                "/song/tag/search",
                &TagSearchReq {
                    query: "原教".to_string(),
                },
            )
            .await
            .parse_resp()
            .await
            .unwrap();

        let first_tag = resp.result.first().unwrap();
        assert_eq!("原教旨", first_tag.name);
        assert_eq!(None, first_tag.description);
    }).await
}

async fn click_farming_likes(env: &TestEnvironment, song_id: i64, number: i64) {
    // Test bench for likes
    let start = SystemTime::now();
    let mut handles = vec![];

    for _ in 0..number {
        let handle = tokio::spawn({
            let conn = env.redis.clone();
            let pool = env.pool.clone();
            async move {
                song_like::like(&conn, &pool, song_id, rand::random(), None).await.unwrap();
            }
        });
        handles.push(handle);
    }
    join_all(handles).await;
    println!("Spend {:.2} secs to execute 60000 likes", start.elapsed().unwrap().as_secs_f64());
}

#[tokio::test]
async fn test_page_by_users() {
    with_test_environment(|mut env| async move {
        // Test first page with small page size
        let resp: RecentResp = env.api.get_query(
            "/song/page_by_user",
            &PageByUserReq {
                user_id: 100004,
                page: Some(0),
                size: Some(20),
            },
        )
            .await
            .parse_resp()
            .await
            .unwrap();
        assert!(resp.songs.iter().all(|x| x.uploader_uid == 100004));

        println!("First page: {:#?}", resp.songs);

        // Test second page
        let resp2: RecentResp = env.api.get_query(
            "/song/page_by_user",
            &PageByUserReq {
                user_id: 100004,
                page: Some(1),
                size: Some(20),
            },
        ).await.parse_resp().await.unwrap();
        // Assert no songs appear in both pages
        let resp2_ids = resp2.songs.iter().map(|song| song.id).collect::<std::collections::HashSet<_>>();
        assert!(resp.songs.iter().all(|song1| !resp2_ids.contains(&song1.id)));

        println!("Second page: {:#?}", resp2.songs);
    }).await
}

#[tokio::test]
async fn test_likes() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        let songs: RecentResp = env.api
            .get_query("/song/recent_v2", &RecentReq { cursor: None, limit: None, after: None }).await
            .parse_resp().await.unwrap();
        let song = songs.songs.first().unwrap();

        let like_req = LikeReq {
            song_id: song.id,
            playback_position_secs: Some(123),
        };

        assert_is_ok(env.api.post("/song/likes/like", &like_req).await).await;

        let status: LikeStatusResp = env.api
            .get_query("/song/likes/status", &like_req).await
            .parse_resp().await.unwrap();
        assert!(status.liked, "song should be liked after /like");

        let page: MyLikesResp = env.api
            .get_query(
                "/song/likes/page_my_likes",
                &MyLikesReq {
                    page_index: 0,
                    page_size: 10,
                },
            ).await.parse_resp().await.unwrap();
        assert_eq!(page.page_index, 0);
        assert_eq!(page.page_size, 10);
        assert_eq!(page.total, 1);
        assert_eq!(page.data.len(), 1);
        assert_eq!(page.data[0].song_data.id, song.id);

        assert_is_ok(env.api.post("/song/likes/unlike", &like_req).await).await;

        let status: LikeStatusResp = env.api.get_query("/song/likes/status", &like_req)
            .await.parse_resp().await.unwrap();
        assert!(!status.liked, "song should be unliked after /unlike");

        let page: MyLikesResp = env
            .api
            .get_query(
                "/song/likes/page_my_likes",
                &MyLikesReq {
                    page_index: 0,
                    page_size: 10,
                },
            )
            .await.parse_resp().await.unwrap();
        assert_eq!(page.total, 0);
        assert!(page.data.is_empty());
    }).await
}

#[tokio::test]
async fn test_likes_validation() {
    with_test_environment(|mut env| async move {
        let _user = with_new_random_test_user(&mut env).await;

        let invalid_index = env
            .api
            .get_query(
                "/song/likes/page_my_likes",
                &MyLikesReq {
                    page_index: -1,
                    page_size: 10,
                },
            )
            .await;
        assert_is_err(invalid_index).await;

        let invalid_size = env
            .api
            .get_query(
                "/song/likes/page_my_likes",
                &MyLikesReq {
                    page_index: 0,
                    page_size: 0,
                },
            )
            .await;
        assert_is_err(invalid_size).await;
    }).await
}