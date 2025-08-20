use crate::common::{assert_is_ok, CommonParse};
use hachimi_world_server::web::routes::playlist::{AddSongReq, CreatePlaylistReq, CreatePlaylistResp, DetailReq, DetailResp, ListResp};
use crate::common::auth::with_new_random_test_user;
use crate::common::with_test_environment;

mod common;

#[tokio::test]
async fn test_playlist() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        // Create a playlist with invalid input
        let resp = env.api.post("/playlist/create", &CreatePlaylistReq {
            name: "Test Long Name".repeat(20),
            description: None,
            is_public: false,
        }).await.parse_resp::<CreatePlaylistResp>().await;
        assert!(resp.is_err(), "CreatePlaylist with long name should return an error");
        assert_eq!(resp.err().unwrap().code, "name_too_long");

        // Create a playlist without cover
        let playlist_resp = env.api.post("/playlist/create", &CreatePlaylistReq {
            name: "Test Playlist".to_string(),
            description: None,
            is_public: false,
        }).await.parse_resp::<CreatePlaylistResp>().await.unwrap();

        let playlist_id = playlist_resp.id;

        let songs_to_add = vec![1, 2, 3, 4, 5];
        for x in songs_to_add {
            let r = env.api.post("/playlist/add_song", &AddSongReq {
                playlist_id,
                song_id: x,
            }).await.parse_resp::<()>().await;
            assert!(r.is_ok())
        }

        let playlist_resp = env.api.get("/playlist/list").await.parse_resp::<ListResp>().await.unwrap();
        assert_eq!(1, playlist_resp.playlists.len());

        let playlist = playlist_resp.playlists.first().unwrap();
        assert_eq!("Test Playlist", playlist.name);
        assert_eq!(None, playlist.description);
        assert_eq!(false, playlist.is_public);
        assert_eq!(5, playlist.songs_count);
        
        let detail = env.api.get_query("/playlist/detail_private", &DetailReq {
            id: playlist.id
        }).await.parse_resp::<DetailResp>().await.unwrap();
        assert_eq!(&playlist.name, &detail.playlist_info.name);
        
        assert_eq!(5, detail.songs.len());
        println!("{:#?}", detail.songs);

        // TODO[test]: Add test for create many playlist
        ()
    }).await;
}

#[tokio::test]
async fn test_invalid_input() {
    with_test_environment(|mut env| async move {
        let user = with_new_random_test_user(&mut env).await;

        // Create a playlist with invalid input
        let resp = env.api.post("/playlist/create", &CreatePlaylistReq {
            name: "Test Long Name".repeat(20),
            description: None,
            is_public: false,
        }).await.parse_resp::<CreatePlaylistResp>().await;
        assert!(resp.is_err(), "CreatePlaylist with long name should return an error");
        assert_eq!(resp.err().unwrap().code, "name_too_long");

        // Create a playlist with invalid input
        let resp = env.api.post("/playlist/create", &CreatePlaylistReq {
            name: "Test Name".to_string(),
            description: Some("Test description".repeat(100)),
            is_public: false,
        }).await.parse_resp::<CreatePlaylistResp>().await;
        assert!(resp.is_err(), "CreatePlaylist with long description should return an error");
        assert_eq!(resp.err().unwrap().code, "description_too_long");
    }).await
}