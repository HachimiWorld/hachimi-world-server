use crate::audio::ParseError;
use crate::db::CrudDao;
use crate::db::song::{ISongDao, Song, SongDao, SongOriginInfo, SongProductionCrew};
use crate::db::user::UserDao;
use crate::web::jwt::Claims;
use crate::web::result::WebError;
use crate::web::result::WebResponse;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{audio, err, ok};
use anyhow::{anyhow, Context};
use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::routing::{get, post};
use chrono::Utc;
use rand::Rng;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};
use crate::db::song_tag::{ISongTagDao, SongTag, SongTagDao};

pub fn router() -> Router<AppState> {
    Router::new()
        // Core operations
        .route("/upload_audio_file", post(upload_audio_file))
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024)) // 20MB
        .route("/upload_cover_image", post(upload_cover_image))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024)) // 20MB
        .route("/detail", get(detail))
        .route("/publish", post(publish))
        .route("/delete", post(delete))
        // Discovery
        .route("/search", get(search))
        .route("/recommend", get(recommend))
        // .route("/trending", get(trending))
        // .route("/recent", get(recent))
        // User interactions
        .route("/like", post(like))
        .route("/play", post(play))
        // Tags
        .route("/tag/create", post(tag_create))
        .route("/tag/search", get(tag_search))
        // .route("/tag/report_merge", post(tag_report_merge))
        // .route("/tag/commit_translation", post())
        // Playlists
        // .rotue("/playlist/create", post(create_playlist))
        // .route("/playlist/update", post(update_playlist))
        // .route("/playlist/delete", post(delete_playlist))
        // .route("/playlist/add_song", post(add_song_to_playlist))
        // .route("/playlist/remove_song", post(remove_song_from_playlist))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailReq {
    /// Actually displayed id
    pub id: String,
}

async fn detail(params: Query<DetailReq>) {

}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishReq {
    pub song_temp_id: String,
    pub cover_temp_id: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub lyrics: String,
    pub tag_ids: Vec<i64>,
    pub creation_info: CreationInfo,
    pub production_crew: Vec<ProductionItem>,
    pub external_links: Vec<ExternalLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreationInfo {
    /// 0: original, 1: derivative work, 2: tertiary work
    pub creation_type: i32,
    pub origin_info: Option<CreationTypeInfo>,
    pub derivative_info: Option<CreationTypeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreationTypeInfo {
    // If `song_id` is Some, the rest fields could be None
    pub song_display_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductionItem {
    pub role: String,
    pub uid: Option<i64>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalLink {
    pub platform: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResp {
    pub song_display_id: String,
}

async fn publish(
    claims: Claims,
    mut state: State<AppState>,
    req: Json<PublishReq>,
) -> WebResult<PublishResp> {
    let uid = claims.uid();

    // Validate input
    // Validate creation_type
    if req.creation_info.creation_type == 1 && req.creation_info.origin_info.is_none() {
        err!(
            "missing_origin_info",
            "Missing origin info for derivative song"
        );
    }
    if req.creation_info.creation_type == 2 && req.creation_info.derivative_info.is_none() {
        err!(
            "missing_origin_info",
            "Missing derivative info for derivative song"
        );
    }

    let user_dao = UserDao::new(state.sql_pool.clone());
    
    // Processing data
    let user = user_dao
        .get_by_id(uid)
        .await?
        .ok_or_else(|| WebError::common("user_not_found", "User not found"))?;

    let song_dao = SongDao::new(state.sql_pool.clone());
    let song_temp_data: String = state
        .redis_conn
        .get(build_temp_key(&req.song_temp_id))
        .await?;
    let song_temp_data: SongTempData = serde_json::from_str(&song_temp_data)?;

    let cover_url: String = state
        .redis_conn
        .get(build_image_temp_key(&req.cover_temp_id))
        .await?;

    let display_id = loop {
        let id = generate_song_display_id();
        if song_dao.get_by_display_id(&id).await?.is_some() {
            continue;
        }
        break id;
    };
    let now = Utc::now();

    let song = Song {
        id: 0,
        display_id: display_id.to_string(),
        title: req.title.to_string(),
        artist: user.username.to_string(),
        file_url: song_temp_data.file_url.to_string(),
        cover_art_url: cover_url.to_string(),
        lyrics: req.lyrics.to_string(),
        duration_seconds: song_temp_data.duration_secs as i32, // Fuck the num type
        uploader_uid: user.id,
        creation_type: req.creation_info.creation_type,
        play_count: 0,
        like_count: 0,
        is_private: false,
        release_time: now,
        create_time: now,
        update_time: now, // Do we really need three time data?
    };

    let song_id = song_dao.insert(&song).await?;
    
    let mut song_origin_infos = Vec::new();
    for x in [
        &req.creation_info.origin_info,
        &req.creation_info.derivative_info,
    ] {
        if let Some(item) = x {
            // Validate, must set one of the id or title
            if item.song_display_id.is_none() && item.title.is_none() {
                err!("title_missed", "Origin info title must be set")
            }
            // Parse internal song id
            let song = if let Some(ref display_id) = item.song_display_id {
                let song = song_dao
                    .get_by_display_id(&display_id)
                    .await?
                    .ok_or_else(|| WebError::common("song_not_found", "Song not found"))?;
                Some(song)
            } else {
                None
            };
            // Add to batch
            song_origin_infos.push(SongOriginInfo {
                id: 0,
                song_id,
                origin_song_id: song.map(|x| x.id),
                origin_title: item.title.clone(),
                origin_artist: item.artist.clone(),
                origin_url: item.url.clone(),
            });
        }
    }
    song_dao
        .update_song_origin_info(song_id, song_origin_infos)
        .await?;

    let mut production_crew = Vec::new();
    for x in &req.production_crew {
        let user = if let Some(uid) = x.uid {
            let song = user_dao
                .get_by_id(uid)
                .await?
                .ok_or_else(|| WebError::common("crew_user_not_found", "Crew user not found"))?;
            Some(song)
        } else {
            None
        };

        production_crew.push(SongProductionCrew {
            id: 0,
            song_id,
            role: x.role.clone(),
            uid: user.map(|x| x.id),
            person_name: x.name.clone(),
        })
    }
    song_dao.update_song_production_crew(song_id, production_crew).await?;

    // Update tags, should we validate these tags?
    song_dao.update_song_tags(song_id, req.tag_ids.clone()).await?;

    ok!(PublishResp {
        song_display_id: display_id
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadAudioFileResp {
    pub temp_id: String,
    pub duration_secs: u64,
    pub title: Option<String>,
    pub bitrate: Option<String>,
    pub artist: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongTempData {
    pub file_url: String,
    pub duration_secs: u64,
}

async fn upload_audio_file(
    claims: Claims,
    mut state: State<AppState>,
    mut multipart: Multipart,
) -> WebResult<UploadAudioFileResp> {
    // 1. Receive streams
    let data_field = multipart
        .next_field()
        .await?
        .with_context(|| "No data field found")?;

    let file_name = data_field.file_name().map(|x| x.to_string());

    // TODO[opt](song): decode and receive in parallel
    let bytes = data_field.bytes().await?;
    let cursor = Cursor::new(bytes.clone());

    // 2. Validate metadata
    let metadata =
        match audio::parse_and_validate(Box::new(cursor), file_name.as_ref().map(|x| x.as_str())) {
            Ok(v) => v,
            Err(err) => match err {
                ParseError::FormatUnsupported => {
                    err!("format_unsupported", "Audio format not supported")
                }
                ParseError::TrackNotFound => err!("track_not_found", "Audio track not found"),
                ParseError::MetadataNotFound(key) => err!(
                    "metadata_not_found",
                    &format!("Metadata {key} not found in audio")
                ),
                ParseError::ParsingDurationError => err!("parsing_duration_error", "Failed to parse duration"),
                ParseError::Parse(err) => {
                    tracing::error!("Error parsing audio: {:?}", err);
                    err!("parse_error", "Error parsing audio")
                }
            },
        };

    // 3. Upload to s3
    // Generate a random filename
    let file_name = format!("{}.{}", uuid::Uuid::new_v4(), metadata.format);
    let result = state
        .file_host
        .upload(bytes, &format!("songs/{}", file_name))
        .await?;

    let temp_id = uuid::Uuid::new_v4().to_string();
    let data = serde_json::to_string(&SongTempData {
        file_url: result.public_url.to_string(),
        duration_secs: metadata.duration_secs,
    })?;
    let _: () = state
        .redis_conn
        .set_ex(build_temp_key(&temp_id), data, 3600)
        .await?;

    ok!(UploadAudioFileResp {
        temp_id: temp_id,
        title: metadata.title,
        duration_secs: metadata.duration_secs,
        bitrate: None,
        artist: None,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadImageResp {
    pub temp_id: String,
}

async fn upload_cover_image(
    claims: Claims,
    mut state: State<AppState>,
    mut multipart: Multipart,
) -> WebResult<UploadImageResp> {
    let user_dao = UserDao::new(state.sql_pool.clone());
    let mut user = if let Some(x) = user_dao.get_by_id(claims.uid()).await? {
        x
    } else {
        err!("not_found", "User not found")
    };

    let data_field = multipart
        .next_field()
        .await?
        .with_context(|| "No data field found")?;
    let bytes = data_field.bytes().await?;

    let start = std::time::Instant::now();

    // Validate image
    if bytes.len() > 8 * 1024 * 1024 {
        err!("image_too_large", "Image size must be less than 8MB");
    }
    let format = ImageReader::new(Cursor::new(bytes.clone()))
        .with_guessed_format()
        .map_err(|_| WebError::common("invalid_image", "Invalid image"))?
        .format()
        .ok_or_else(|| WebError::common("invalid_image", "Invalid image"))?;
    
    let format_ext = match format {
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Avif => {
            format.extensions_str().first().ok_or_else(|| anyhow!("Cannot get extension name"))?
        }
        _ => err!("format_unsupported", "Image format unsupported")
    };
    
    // Upload image
    let sha1 = openssl::sha::sha1(&bytes);
    let filename = format!("images/cover/{}.{}", hex::encode(sha1), format_ext);
    let result = state.file_host.upload(bytes, &filename).await?;
    let temp_id = uuid::Uuid::new_v4().to_string();
    let _: () = state
        .redis_conn
        .set_ex(build_image_temp_key(&temp_id), result.public_url, 3600)
        .await?;
    
    ok!(UploadImageResp { temp_id })
}

fn build_temp_key(temp_id: &str) -> String {
    let key = format!("song_upload:temp:{}", temp_id);
    key
}
fn build_image_temp_key(temp_id: &str) -> String {
    let key = format!("songs_upload:cover_temp:{}", temp_id);
    key
}
pub struct DeleteReq {
    pub song_id: i64,
}

async fn delete() -> WebResult<()> {
    err!("no_impl", "Not implemented yet");
}

async fn search() -> WebResult<()> {
    err!("no_impl", "Not implemented yet")
}

async fn recommend() {}

async fn like() {}

async fn play() {}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagSearchReq {
    pub query: String
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagSearchResp {
    pub result: Vec<TagItem>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagItem {
    pub id: i64,
    pub name: String,
    pub description: Option<String>
}

async fn tag_search(
    claims: Claims, // Consider removing auth
    state: State<AppState>,
    req: Query<TagSearchReq>,
) -> WebResult<TagSearchResp> {
    let song_tag_dao = SongTagDao::new(state.sql_pool.clone());
    // TODO[opt](tag): Replace with real full-text search
    let result = song_tag_dao.search_by_prefix(&req.query).await?
        .into_iter().map(|x| TagItem {
        id: x.id,
        name: x.name,
        description: x.description,
    }).collect();
    ok!(TagSearchResp {result})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagCreateReq {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagCreateResp {
    pub id: i64
}

async fn tag_create(claims: Claims, state: State<AppState>, req: Json<TagCreateReq>) -> WebResult<TagCreateResp> {
    // TODO[feat](song-tag): Need audit procedure
    if req.name.is_empty() || req.name.chars().count() > 10 {
        err!("invalid_name", "Invalid name")
    }

    let song_tag_dao = SongTagDao::new(state.sql_pool.clone());
    if song_tag_dao.get_by_name(req.name.as_str()).await?.is_some() {
        err!("name_exists", "Tag name already exists")
    }

    let id = song_tag_dao.insert(
        &SongTag {
            id: 0,
            name: req.name.clone(),
            description: req.description.clone(),
            is_active: true,
            create_time: Utc::now(),
            update_time: Utc::now(),
        }
    ).await?;
    ok!(TagCreateResp { id })
}

/// Pattern: JM-AAAA-000
fn generate_song_display_id() -> String {
    let mut rng = rand::rng();

    // 生成4个随机大写字母
    let letters: String = (0..4)
        .map(|_| rng.random_range(b'A'..=b'Z') as char)
        .collect();

    // 生成3个随机数字
    let numbers: String = (0..3)
        .map(|_| rng.random_range(b'0'..=b'9') as char)
        .collect();

    // 组合成目标格式
    format!("JM-{}-{}", letters, numbers)
}

