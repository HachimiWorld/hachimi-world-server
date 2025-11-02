mod common;

use crate::common::{assert_is_ok, CommonParse};
use crate::common::{with_test_environment, TestEnvironment};
use futures::future::join_all;
use hachimi_world_server::service::song_like;
use hachimi_world_server::web::routes::song::{DetailReq, DetailResp, RecentResp, SearchReq, SearchResp, SongListResp, TagCreateReq, TagSearchReq, TagSearchResp, PageByUserReq};
use std::time::SystemTime;
use crate::common::auth::with_new_random_test_user;

#[tokio::test]
async fn test_get_likes() {
    with_test_environment(|mut env| async move {
        // Get likes
        let resp: DetailResp = env.api.get_query("/song/detail", &DetailReq { id: "JM-IOEW-474".to_string() }).await.parse_resp().await.unwrap();
        println!("{:#?}", resp);
    }).await;
}

#[tokio::test]
async fn test_discover_songs() {
    with_test_environment(|mut env| async move {
        let test = with_new_random_test_user(&mut env).await;
        /*let resp: SongListResp = env.api.get("/song/hot").await.parse_resp().await.unwrap();
        println!("hot: {:?}", resp.song_ids);

        let resp: SongListResp = env.api.get("/song/recent").await.parse_resp().await.unwrap();
        println!("recent: {:?}", resp.song_ids);

        let resp: RecentResp = env.api.get("/song/recent_v2").await.parse_resp().await.unwrap();
        println!("recent: {:?}", resp.songs);*/

        let resp: RecentResp = env.api.get("/song/recommend").await.parse_resp().await.unwrap();
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
                song_like::like(&conn, &pool, song_id, rand::random()).await.unwrap();
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
        assert!(resp.songs.iter().all(|song1|
            !resp2.songs.iter().any(|song2| song1.id == song2.id)
        ));

        println!("Second page: {:#?}", resp2.songs);
    }).await
}