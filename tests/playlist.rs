use crate::common::auth::{with_new_random_test_user, with_test_contributor_user};
use crate::common::publish::create_approved_song;
use crate::common::{with_test_environment, CommonParse, TestEnvironment};
use hachimi_world_server::db::playlist::PlaylistId;
use hachimi_world_server::db::song::SongId;
use hachimi_world_server::web::routes::playlist::{
    AddFavoriteReq, AddSongReq, ChangeOrderReq, CheckFavoriteReq, CheckFavoriteResp,
    CreatePlaylistReq, CreatePlaylistResp, DetailReq, DetailResp, ListContainingReq,
    ListContainingResp, ListResp, PageFavoritesReq, PageFavoritesResp, RemoveSongReq, SearchReq,
    SearchResp, SetCoverReq,
};
use image::{ImageFormat, Rgb, RgbImage};
use reqwest::multipart::{Form, Part};
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
                create_approved_song(&mut env, &owner_token, &contributor_token, title).await.id,
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
        let song_id = create_approved_song(&mut env, &owner_token, &contributor_token, "Contained Song").await.id;

        add_song_to_playlist(&env, playlist_id, song_id).await;

        let containing = env.api.get_query("/playlist/list_containing", &ListContainingReq { song_id }).await
            .parse_resp::<ListContainingResp>().await.unwrap();
        assert_eq!(containing.playlist_ids, vec![playlist_id]);

        let not_containing = env.api.get_query("/playlist/list_containing", &ListContainingReq { song_id: SongId(0) }).await
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
        let song_id = create_approved_song(&mut env, &owner_token, &contributor_token, "Duplicate Song").await.id;

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
        let not_favorite = env.api.get_query("/playlist/favorite/check", &CheckFavoriteReq { playlist_id: PlaylistId(0) }).await
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

#[tokio::test]
async fn test_playlist_default_cover() {
    with_test_environment(|mut env| async move {
        let owner = with_new_random_test_user(&mut env).await;
        let contributor = with_test_contributor_user(&mut env).await;
        let owner_token = owner.token.access_token.clone();
        let contributor_token = contributor.token.access_token.clone();

        let mut songs = Vec::new();
        for title in ["Cover A", "Cover B", "Cover C"] {
            let id = create_approved_song(&mut env, &owner_token, &contributor_token, title).await.id;
            set_song_cover(&env, id, &format!("https://example.com/{title}.webp")).await;
            songs.push(id);
        }
        let cover = |i: usize| Some(format!("https://example.com/Cover {}.webp", ["A", "B", "C"][i]));
        env.api.set_token(owner_token.clone());

        let playlist_id = create_playlist(&env, "Default Cover", None, true).await;
        assert_eq!(playlist_cover(&env, playlist_id).await, None, "No songs, no cover");

        add_song_to_playlist(&env, playlist_id, songs[0]).await;
        assert_eq!(playlist_cover(&env, playlist_id).await, cover(0));

        // The most recently added song wins, wherever it is in the order
        add_song_to_playlist(&env, playlist_id, songs[1]).await;
        env.api.post("/playlist/change_order", &ChangeOrderReq { playlist_id, song_id: songs[1], target_order: 0 })
            .await.parse_resp::<()>().await.unwrap();
        assert_eq!(playlist_cover(&env, playlist_id).await, cover(1));
        let detail = env.api.get_query("/playlist/detail", &DetailReq { id: playlist_id }).await
            .parse_resp::<DetailResp>().await.unwrap();
        assert_eq!(detail.playlist_info.cover_url, cover(1));

        // Hiding a song doesn't update the cover by itself, but hidden songs are skipped when it's
        // picked again
        set_song_hidden(&env, songs[1], true).await;
        assert_eq!(playlist_cover(&env, playlist_id).await, cover(1));
        add_song_to_playlist(&env, playlist_id, songs[2]).await;
        assert_eq!(playlist_cover(&env, playlist_id).await, cover(2));
        remove_song_from_playlist(&env, playlist_id, songs[2]).await;
        assert_eq!(playlist_cover(&env, playlist_id).await, cover(0));

        remove_song_from_playlist(&env, playlist_id, songs[0]).await;
        assert_eq!(playlist_cover(&env, playlist_id).await, None, "Only a hidden song is left");
    }).await;
}

#[tokio::test]
async fn test_playlist_custom_cover_is_kept() {
    with_test_environment(|mut env| async move {
        let owner = with_new_random_test_user(&mut env).await;
        let contributor = with_test_contributor_user(&mut env).await;
        let owner_token = owner.token.access_token.clone();
        let contributor_token = contributor.token.access_token.clone();
        let song_id = create_approved_song(&mut env, &owner_token, &contributor_token, "Not The Cover").await.id;
        set_song_cover(&env, song_id, "https://example.com/song.webp").await;
        env.api.set_token(owner_token.clone());

        let playlist_id = create_playlist(&env, "Custom Cover", None, false).await;
        let mut png = Vec::new();
        RgbImage::from_pixel(300, 200, Rgb([128, 128, 128]))
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png).unwrap();
        let json = serde_json::to_string(&SetCoverReq { playlist_id }).unwrap();
        env.api.post_raw("/playlist/set_cover")
            .multipart(Form::new().text("json", json).part("file", Part::bytes(png).file_name("cover.png")))
            .send().await.unwrap()
            .parse_resp::<()>().await.unwrap();
        let custom = playlist_cover(&env, playlist_id).await;
        assert!(custom.is_some());

        add_song_to_playlist(&env, playlist_id, song_id).await;
        assert_eq!(playlist_cover(&env, playlist_id).await, custom);
        remove_song_from_playlist(&env, playlist_id, song_id).await;
        assert_eq!(playlist_cover(&env, playlist_id).await, custom);
    }).await;
}

/// The cover of one of the user's own playlists, from `/playlist/list`.
async fn playlist_cover(env: &TestEnvironment, playlist_id: PlaylistId) -> Option<String> {
    let list = env.api.get("/playlist/list").await.parse_resp::<ListResp>().await.unwrap();
    list.playlists.into_iter().find(|x| x.id == playlist_id).unwrap().cover_url
}

async fn set_song_cover(env: &TestEnvironment, song_id: SongId, cover_url: &str) {
    sqlx::query("UPDATE songs SET cover_art_url = $1 WHERE id = $2")
        .bind(cover_url).bind(song_id.0)
        .execute(&env.pool).await.unwrap();
}

async fn set_song_hidden(env: &TestEnvironment, song_id: SongId, hidden: bool) {
    sqlx::query("UPDATE songs SET is_hidden = $1 WHERE id = $2")
        .bind(hidden).bind(song_id.0)
        .execute(&env.pool).await.unwrap();
}

async fn remove_song_from_playlist(env: &TestEnvironment, playlist_id: PlaylistId, song_id: SongId) {
    env.api.post("/playlist/remove_song", &RemoveSongReq { playlist_id, song_id })
        .await.parse_resp::<()>().await.unwrap();
}

async fn create_playlist(
    env: &TestEnvironment,
    name: &str,
    description: Option<&str>,
    is_public: bool,
) -> PlaylistId {
    env.api.post(
        "/playlist/create",
        &CreatePlaylistReq {
            name: name.to_string(),
            description: description.map(str::to_string),
            is_public,
        },
    ).await.parse_resp::<CreatePlaylistResp>().await.unwrap().id
}

async fn add_song_to_playlist(env: &TestEnvironment, playlist_id: PlaylistId, song_id: SongId) {
    env.api.post(
        "/playlist/add_song",
        &AddSongReq {
            playlist_id,
            song_id,
        },
    ).await.parse_resp::<()>().await.unwrap();
}

async fn wait_for_playlist_search_hit(
    env: &TestEnvironment,
    query: &str,
    playlist_id: PlaylistId,
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
