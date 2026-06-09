use crate::common::auth::{with_new_random_test_user, with_test_contributor_user};
use crate::common::publish::publish_template;
use crate::common::{with_test_environment, CommonParse, TestEnvironment};
use hachimi_world_server::db::song::{ISongDao, SongDao};
use hachimi_world_server::service::song::generate_song_display_id;
use hachimi_world_server::web::routes::playlist::{
    AddFavoriteReq, AddSongReq, ChangeOrderReq, CheckFavoriteReq, CheckFavoriteResp,
    CreatePlaylistReq, CreatePlaylistResp, DetailReq, DetailResp, ListContainingReq,
    ListContainingResp, ListResp, PageFavoritesReq, PageFavoritesResp, SearchReq, SearchResp,
};
use hachimi_world_server::web::routes::publish::jmid::JmidGetNextResp;
use hachimi_world_server::web::routes::publish::review::ApproveReviewReq;
use hachimi_world_server::web::routes::publish::PublishResp;
use tokio::time::{sleep, Duration, Instant};

mod common;

#[tokio::test]
async fn test_playlist_detail_and_reorder_are_isolated() {
    with_test_environment(|mut env| async move {
        let owner = with_new_random_test_user(&mut env).await;
        let contributor = with_test_contributor_user(&mut env).await;
        let owner_token = owner.token.access_token.clone();
        let contributor_token = contributor.token.access_token.clone();
        env.api.set_token(owner_token.clone());

        // Create test playlist
        let playlist_id = create_playlist(&env, "Test Playlist", Some("Created inside the isolated test environment"), false).await;

        // Add songs to the playlist
        let mut song_ids = Vec::new();
        for title in ["Playlist Song 1", "Playlist Song 2", "Playlist Song 3", "Playlist Song 4"] {
            song_ids.push(
                create_approved_song(&mut env, &owner_token, &contributor_token, title).await,
            );
        }

        for song_id in &song_ids {
            add_song_to_playlist(&env, playlist_id, *song_id).await;
        }

        // Test song count changed after adding songs
        let list = env.api.get("/playlist/list").await.parse_resp::<ListResp>().await.unwrap();
        assert_eq!(1, list.playlists.len());

        let playlist = list.playlists.first().unwrap();
        assert_eq!(playlist.id, playlist_id);
        assert_eq!(playlist.name, "Test Playlist");
        assert_eq!(
            playlist.description.as_deref(),
            Some("Created inside the isolated test environment")
        );
        assert!(!playlist.is_public);
        assert_eq!(playlist.songs_count, song_ids.len() as i64);

        // Get songs in the playlist and verify order [0, 1, 2, 3]
        let detail = env.api
            .get_query("/playlist/detail_private", &DetailReq { id: playlist_id }).await
            .parse_resp::<DetailResp>().await.unwrap();
        assert_eq!(detail.playlist_info.id, playlist_id);
        assert_eq!(detail.songs.len(), song_ids.len());
        assert_eq!(
            detail.songs.iter().map(|song| song.song_id).collect::<Vec<_>>(),
            song_ids,
            "The order should be the same as we added"
        );

        // Move the 3rd song to the first position
        env.api.post(
            "/playlist/change_order",
            &ChangeOrderReq {
                playlist_id,
                song_id: song_ids[2],
                target_order: 0,
            },
        ).await.parse_resp::<()>().await.unwrap();

        // Check the order should be [2, 0, 1, 3]
        let reordered = env.api
            .get_query("/playlist/detail_private", &DetailReq { id: playlist_id }).await
            .parse_resp::<DetailResp>().await.unwrap();
        assert_eq!(
            reordered
                .songs
                .iter()
                .map(|song| song.song_id)
                .collect::<Vec<_>>(),
            vec![song_ids[2], song_ids[0], song_ids[1], song_ids[3]]
        );

        // Move the 2nd song (song_ids[0]) to the last position again
        env.api.post(
            "/playlist/change_order",
            &ChangeOrderReq {
                playlist_id,
                song_id: song_ids[0],
                target_order: 3,
            },
        ).await.parse_resp::<()>().await.unwrap();

        // Check the order should be [2, 1, 3, 0]
        let reordered_again = env.api
            .get_query("/playlist/detail_private", &DetailReq { id: playlist_id }).await
            .parse_resp::<DetailResp>().await.unwrap();
        assert_eq!(
            reordered_again.songs.iter().map(|song| song.song_id).collect::<Vec<_>>(),
            vec![song_ids[2], song_ids[1], song_ids[3], song_ids[0]]
        );
    }).await;
}

#[tokio::test]
async fn test_playlist_list_containing() {
    with_test_environment(|mut env| async move {
        let owner = with_new_random_test_user(&mut env).await;
        let contributor = with_test_contributor_user(&mut env).await;
        let owner_token = owner.token.access_token.clone();
        let contributor_token = contributor.token.access_token.clone();
        env.api.set_token(owner_token.clone());

        let playlist_id = create_playlist(&env, "Containing Playlist", Some("Created for list_containing test"), false).await;
        let song_id = create_approved_song(&mut env, &owner_token, &contributor_token, "Contained Song").await;

        add_song_to_playlist(&env, playlist_id, song_id).await;

        let containing = env.api.get_query("/playlist/list_containing", &ListContainingReq { song_id }).await
            .parse_resp::<ListContainingResp>().await.unwrap();
        assert_eq!(containing.playlist_ids, vec![playlist_id]);

        let not_containing = env.api.get_query("/playlist/list_containing", &ListContainingReq { song_id: 0 }).await
            .parse_resp::<ListContainingResp>().await.unwrap();
        assert!(not_containing.playlist_ids.is_empty());
    }).await;
}

#[tokio::test]
async fn test_create_playlist_validates_input() {
    with_test_environment(|mut env| async move {
        let _owner = with_new_random_test_user(&mut env).await;

        let resp = env.api.post(
            "/playlist/create",
            &CreatePlaylistReq {
                name: "Test Long Name".repeat(20),
                description: None,
                is_public: false,
            },
        ).await.parse_resp::<CreatePlaylistResp>().await;
        assert_eq!(resp.unwrap_err().code, "invalid_name");

        let resp = env.api.post(
            "/playlist/create",
            &CreatePlaylistReq {
                name: "Test Name".to_string(),
                description: Some("Test description".repeat(100)),
                is_public: false,
            },
        ).await.parse_resp::<CreatePlaylistResp>().await;
        assert_eq!(resp.unwrap_err().code, "description_too_long");
    }).await
}

#[tokio::test]
async fn test_add_song_rejects_duplicates() {
    with_test_environment(|mut env| async move {
        let owner = with_new_random_test_user(&mut env).await;
        let contributor = with_test_contributor_user(&mut env).await;
        let owner_token = owner.token.access_token.clone();
        let contributor_token = contributor.token.access_token.clone();
        env.api.set_token(owner_token.clone());

        let playlist_id = create_playlist(&env, "Duplicate Guard", None, false).await;
        let song_id = create_approved_song(&mut env, &owner_token, &contributor_token, "Duplicate Song").await;

        add_song_to_playlist(&env, playlist_id, song_id).await;

        let resp = env.api.post(
            "/playlist/add_song",
            &AddSongReq {
                playlist_id,
                song_id,
            },
        ).await.parse_resp::<()>().await;
        assert_eq!(resp.unwrap_err().code, "song_existed");
    }).await;
}

#[tokio::test]
async fn test_search_only_returns_public_playlists() {
    with_test_environment(|mut env| async move {
        let _owner = with_new_random_test_user(&mut env).await;

        let query = format!("playlist-search-{}", uuid::Uuid::new_v4().simple());
        let public_id = create_playlist(&env, "Searchable Playlist", Some(&query), true).await;
        let private_id = create_playlist(&env, "Hidden Playlist", Some(&query), false).await;

        let search = wait_for_playlist_search_hit(&env, &query, public_id).await;
        assert!(search.hits.iter().any(|playlist| playlist.id == public_id));
        assert!(search.hits.iter().all(|playlist| playlist.id != private_id));
        assert_eq!(search.hits.len(), 1);
    }).await;
}

#[tokio::test]
async fn test_favorite_playlists() {
    with_test_environment(|mut env| async move {
        // User1: Create playlist A and B
        let _owner = with_new_random_test_user(&mut env).await;
        let playlist_a = create_playlist(&env, "Favorite A", None, true).await;
        let playlist_b = create_playlist(&env, "Favorite B", None, true).await;

        // User2: Add A and B to favorites
        let _user = with_new_random_test_user(&mut env).await;

        // Should be no favorites at the beginning
        let empty_page = env.api.get_query(
            "/playlist/favorite/page",
            &PageFavoritesReq {
                page_index: 0,
                page_size: 100,
            },
        ).await.parse_resp::<PageFavoritesResp>().await.unwrap();
        assert_eq!(empty_page.total, 0);
        assert!(empty_page.data.is_empty());
        assert_eq!(empty_page.page_index, 0);
        assert_eq!(empty_page.page_size, 50);

        env.api.post("/playlist/favorite/add", &AddFavoriteReq { playlist_id: playlist_a }).await
            .parse_resp::<()>().await.unwrap();
        env.api.post("/playlist/favorite/add", &AddFavoriteReq { playlist_id: playlist_b }).await
            .parse_resp::<()>().await.unwrap();

        // Adding the same playlist again should be rejected
        let duplicate = env.api.post("/playlist/favorite/add", &AddFavoriteReq { playlist_id: playlist_a }).await
            .parse_resp::<()>().await;
        assert_eq!(duplicate.unwrap_err().code, "already_favorited");

        // Check the favorite page should return A and B
        let favorites = env.api.get_query(
            "/playlist/favorite/page",
            &PageFavoritesReq {
                page_index: 0,
                page_size: 50,
            },
        ).await.parse_resp::<PageFavoritesResp>().await.unwrap();
        assert_eq!(favorites.total, 2);
        assert_eq!(favorites.data.len(), 2);
        assert!(favorites.data.iter().any(|item| item.metadata.id == playlist_a && item.order_index == 0));
        assert!(favorites.data.iter().any(|item| item.metadata.id == playlist_b && item.order_index == 1));

        // Check the favorite status of A should be true
        let favorite = env.api.get_query(
            "/playlist/favorite/check",
            &CheckFavoriteReq {
                playlist_id: playlist_a,
            },
        ).await.parse_resp::<CheckFavoriteResp>().await.unwrap();
        assert_eq!(favorite.is_favorite, true);
        assert!(favorite.add_time.is_some());

        // Not favorite playlist should return false
        let not_favorite = env.api.get_query("/playlist/favorite/check", &CheckFavoriteReq { playlist_id: 0 }).await
            .parse_resp::<CheckFavoriteResp>().await.unwrap();
        assert_eq!(not_favorite.is_favorite, false);


        // Remove A from favorites and check again
        env.api.post(
            "/playlist/favorite/remove",
            &CheckFavoriteReq {
                playlist_id: playlist_a,
            },
        ).await.parse_resp::<()>().await.unwrap();

        let removed = env.api.get_query(
            "/playlist/favorite/check",
            &CheckFavoriteReq {
                playlist_id: playlist_a,
            },
        ).await.parse_resp::<CheckFavoriteResp>().await.unwrap();
        assert!(!removed.is_favorite);

        let remaining = env.api.get_query(
            "/playlist/favorite/page",
            &PageFavoritesReq {
                page_index: 0,
                page_size: 50,
            },
        ).await.parse_resp::<PageFavoritesResp>().await.unwrap();
        assert_eq!(remaining.total, 1);
        assert_eq!(remaining.data.len(), 1);
        assert_eq!(remaining.data[0].metadata.id, playlist_b);
    }).await;
}

async fn create_playlist(
    env: &TestEnvironment,
    name: &str,
    description: Option<&str>,
    is_public: bool,
) -> i64 {
    env.api.post(
        "/playlist/create",
        &CreatePlaylistReq {
            name: name.to_string(),
            description: description.map(str::to_string),
            is_public,
        },
    ).await.parse_resp::<CreatePlaylistResp>().await.unwrap().id
}

async fn add_song_to_playlist(env: &TestEnvironment, playlist_id: i64, song_id: i64) {
    env.api.post(
        "/playlist/add_song",
        &AddSongReq {
            playlist_id,
            song_id,
        },
    ).await.parse_resp::<()>().await.unwrap();
}

async fn create_approved_song(
    env: &mut TestEnvironment,
    owner_token: &str,
    contributor_token: &str,
    title: &str,
) -> i64 {
    env.api.set_token(owner_token.to_string());

    let unique = uuid::Uuid::new_v4().simple().to_string();
    let mut req = publish_template(env).await;

    // Get next jmid or random
    let next_jmid = env.api.get("/publish/jmid/get_next").await.parse_resp::<JmidGetNextResp>().await;
    let jmid = match next_jmid {
        Ok(x) => format!("JM-{}", x.jmid),
        Err(_) => unique_jmid()
    };

    req.title = format!("{title}-{unique}");
    req.subtitle = format!("subtitle-{unique}");
    req.description = format!("description-{unique}");
    req.lyrics = format!("lyrics-{unique}");
    req.jmid = Some(jmid);

    let publish_resp = env.api.post("/publish/publish", &req).await
        .parse_resp::<PublishResp>().await.unwrap();

    env.api.set_token(contributor_token.to_string());
    env.api.post(
        "/publish/review/approve",
        &ApproveReviewReq {
            review_id: publish_resp.review_id,
            comment: Some("Approve for playlist integration test".to_string()),
        },
    ).await.parse_resp::<()>().await.unwrap();

    env.api.set_token(owner_token.to_string());
    SongDao::get_by_display_id(&env.pool, &publish_resp.song_display_id).await.unwrap().unwrap().id
}

fn unique_jmid() -> String {
    generate_song_display_id()
}

async fn wait_for_playlist_search_hit(
    env: &TestEnvironment,
    query: &str,
    playlist_id: i64,
) -> SearchResp {
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        let resp = env.api.get_query(
            "/playlist/search",
            &SearchReq {
                q: query.to_string(),
                limit: Some(10),
                offset: Some(0),
                sort_by: None,
                user_id: None,
            },
        ).await.parse_resp::<SearchResp>().await.unwrap();

        if resp.hits.iter().any(|playlist| playlist.id == playlist_id) {
            return resp;
        }

        assert!(
            Instant::now() < deadline,
            "playlist {playlist_id} was not searchable for query {query:?}"
        );
        sleep(Duration::from_millis(100)).await;
    }
}
