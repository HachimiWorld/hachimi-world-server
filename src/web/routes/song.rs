use crate::audio::ParseError;
use crate::db::song::{ISongDao, Song, SongDao, SongOriginInfo, SongProductionCrew};
use crate::db::song_tag::{ISongTagDao, SongTag, SongTagDao};
use crate::db::user::UserDao;
use crate::db::CrudDao;
use crate::service::{recommend, recommend_v2, song, song_like};
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{audio, common, err, ok, search, service};
use anyhow::{anyhow, Context};
use async_backtrace::framed;
use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use chrono::Utc;
use image::{ImageFormat, ImageReader};
use rand::Rng;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use itertools::Itertools;
use crate::db::song_publishing_review::{SongPublishingReview, SongPublishingReviewDao};
use crate::service::song::PublicSongDetail;
use crate::util::IsBlank;
use crate::web::routes::publish::SongPublishReviewData;

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
        .route("/recent", get(recent))
        .route("/recent_v2", get(recent_v2))
        .route("/hot", get(hot))
        // User interactions
        .route("/like", post(like))
        .route("/unlike", post(unlike))
        .route("/play", post(play))
        // Tags
        .route("/tag/create", post(tag_create))
        .route("/tag/search", get(tag_search))
    // .route("/tag/report_merge", post(tag_report_merge))
    // .route("/tag/commit_translation", post())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailReq {
    /// Actually displayed id
    pub id: String,
}

pub type DetailResp = PublicSongDetail;

#[framed]
async fn detail(
    state: State<AppState>,
    params: Query<DetailReq>,
) -> WebResult<DetailResp> {
    let data = song::get_public_detail_with_cache_by_display_id(
        state.redis_conn.clone(),
        &state.sql_pool,
        &params.id
    ).await?;
    match data {
        Some(x) => ok!(x),
        None => err!("not_found", "Song not found")
    }
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
    // TODO: A derivation song can be inspired by many origin songs
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
    pub origin_type: i32,
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
    pub review_id: i64,
    pub song_display_id: String,
}

#[framed]
async fn publish(
    claims: Claims,
    mut state: State<AppState>,
    req: Json<PublishReq>,
) -> WebResult<PublishResp> {
    let uid = claims.uid();
    let user = UserDao::get_by_id(&state.sql_pool, uid).await?.ok_or_else(|| common!("user_not_found", "User not found"))?;

    // Validate input
    // Validate creation_type
    if req.creation_info.creation_type == 1 && req.creation_info.origin_info.is_none() {
        err!("missing_origin_info", "Missing origin info for derivative song");
    }
    if req.creation_info.creation_type == 2 && req.creation_info.derivative_info.is_none() {
        err!("missing_origin_info", "Missing derivative info for derivative song");
    }

    // Validate tags
    let tags = SongTagDao::list_by_ids(&state.sql_pool, &req.tag_ids).await?;
    if tags.len() != req.tag_ids.len() {
        err!("tag_not_found", "Some tags not found");
    }

    // Processing data
    let song_temp_data: Option<String> = state.redis_conn.get(build_temp_key(&req.song_temp_id)).await?;
    let song_temp_data = song_temp_data.ok_or_else(|| common!("invalid_song_temp_id", "Invalid song temp id"))?;
    let song_temp_data: SongTempData = serde_json::from_str(&song_temp_data)?;

    let cover_url: Option<String> = state.redis_conn.get(build_image_temp_key(&req.cover_temp_id)).await?;
    let cover_url = cover_url.ok_or_else(|| common!("invalid_cover_temp_id", "Invalid cover temp id"))?;

    let display_id = loop {
        let id = generate_song_display_id();
        if SongDao::get_by_display_id(&state.sql_pool, &id).await?.is_some() {
            continue;
        }
        break id;
    };
    let now = Utc::now();

    let mut song = Song {
        id: 0,
        display_id: display_id.to_string(),
        title: req.title.to_string(),
        subtitle: req.subtitle.to_string(),
        description: req.description.to_string(),
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
                let song = SongDao::get_by_display_id(&state.sql_pool, &display_id)
                    .await?
                    .ok_or_else(|| common!("song_not_found", "Song not found"))?;
                Some(song)
            } else {
                None
            };
            // Add to batch
            song_origin_infos.push(SongOriginInfo {
                id: 0,
                song_id: 0,
                origin_type: item.origin_type,
                origin_song_id: song.map(|x| x.id),
                origin_title: item.title.clone(),
                origin_artist: item.artist.clone(),
                origin_url: item.url.clone(),
            });
        }
    }

    let mut production_crew = Vec::new();
    for member in &req.production_crew {
        if member.uid.is_none() && member.name.is_none() {
            err!("name_missed", "One of uid or name must be set")
        }

        if let Some(uid) = member.uid {
            let user = UserDao::get_by_id(&state.sql_pool, uid).await?
                .ok_or_else(|| common!("crew_user_not_found", "Crew user not found"))?;
            production_crew.push(SongProductionCrew {
                id: 0,
                song_id: 0,
                role: member.role.clone(),
                uid: Some(user.id),
                person_name: Some(user.username),
            });
        }
        
        if let Some(ref name) = member.name {
            production_crew.push(SongProductionCrew {
                id: 0,
                song_id: 0,
                role: member.role.clone(),
                uid: None,
                person_name: Some(name.clone()),
            });
        }
    }
    
    song.artist = production_crew.iter().map(|x| x.person_name.clone().unwrap_or_else(|| "Unknown".to_string())).join(", ");
    
    let data = SongPublishReviewData {
        song_info: song,
        song_origin_infos: song_origin_infos,
        song_production_crew: production_crew,
        song_tags: tags,
        song_external_links: vec![], // TODO: Support external links
    };
    let review = SongPublishingReview {
        id: 0,
        user_id: claims.uid(),
        song_display_id: display_id.clone(),
        data: serde_json::to_value(data)?,
        submit_time: Utc::now(),
        update_time: Utc::now(),
        review_time: None,
        review_comment: None,
        status: 0,
    };

    let review_id = SongPublishingReviewDao::insert(&state.sql_pool, &review).await?;

    ok!(PublishResp {
        review_id: review_id,
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

#[framed]
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
                    "Metadata {key} not found in audio"
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

#[framed]
async fn upload_cover_image(
    claims: Claims,
    mut state: State<AppState>,
    mut multipart: Multipart,
) -> WebResult<UploadImageResp> {
    let mut user = if let Some(x) = UserDao::get_by_id(&state.sql_pool, claims.uid()).await? {
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
        .map_err(|_| common!("invalid_image", "Invalid image"))?
        .format()
        .ok_or_else(|| common!("invalid_image", "Invalid image"))?;

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

#[framed]
async fn delete() -> WebResult<()> {
    err!("no_impl", "Not implemented yet");
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchReq {
    pub q: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResp {
    pub hits: Vec<SearchSongItem>,
    pub query: String,
    pub processing_time_ms: u64,
    pub total_hits: Option<usize>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSongItem {
    pub id: i64,
    pub display_id: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub artist: String,
    pub duration_seconds: i32,
    pub play_count: i64,
    pub like_count: i64,
    pub cover_art_url: String,
    pub audio_url: String,
}

#[framed]
async fn search(
    state: State<AppState>,
    req: Query<SearchReq>,
) -> WebResult<SearchResp> {
    // Validate search
    if req.q.is_blank() {
        err!("invalid_query", "Query must not be blank")
    }

    let search_query = search::song::SearchQuery {
        q: req.q.clone(),
        limit: req.limit,
        offset: req.offset,
        filter: req.filter.clone(),
    };

    let result = search::song::search_songs(state.meilisearch.as_ref(), &search_query).await?;

    let mut hits = Vec::new();

    for hit in result.hits {
        if let Ok(Some(song)) = SongDao::get_by_id(&state.sql_pool, hit.id).await {
            let like_count = song_like::get_song_likes(&state.redis_conn, &state.sql_pool, song.id).await?;

            hits.push(SearchSongItem {
                id: song.id,
                display_id: song.display_id,
                title: song.title,
                subtitle: song.subtitle,
                description: song.description,
                artist: song.artist,
                duration_seconds: song.duration_seconds,
                play_count: song.play_count,
                like_count: like_count,
                cover_art_url: song.cover_art_url,
                audio_url: song.file_url,
            });
        }
    }

    ok!(SearchResp {
        hits,
        query: result.query,
        processing_time_ms: result.processing_time_ms,
        total_hits: result.hits_info.total_hits,
        limit: result.hits_info.limit,
        offset: result.hits_info.offset,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[deprecated(since = "250831", note = "use /recent_v2 instead")]
pub struct SongListResp {
    pub song_ids: Vec<String>,
}

#[framed]
#[deprecated(since = "250831", note = "use /recent_v2 instead")]
async fn recent(
    state: State<AppState>
) -> WebResult<SongListResp> {
    let songs = recommend::get_recent_songs(&state.redis_conn, &state.sql_pool).await?;
    let ids: Vec<String> = songs.into_iter().map(|x| {
        x.display_id
    }).collect();

    ok!(SongListResp {
        song_ids: ids
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentResp {
    pub songs: Vec<DetailResp>
}

#[framed]
async fn recent_v2(
    state: State<AppState>
) -> WebResult<RecentResp> {
    let songs = recommend_v2::get_recent_songs(state.red_lock.clone(), &state.redis_conn, &state.sql_pool).await?;

    ok!(RecentResp {songs})
}


#[framed]
async fn hot(
    state: State<AppState>
) -> WebResult<SongListResp> {
    let songs = recommend::get_hot_songs(&state.redis_conn, &state.sql_pool).await?;
    let ids: Vec<String> = songs.into_iter().map(|x| {
        x.display_id
    }).collect();

    ok!(SongListResp {
        song_ids: ids
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LikeReq {
    pub song_id: i64,
}

#[framed]
async fn like(
    claims: Claims,
    state: State<AppState>,
    req: Json<LikeReq>,
) -> WebResult<()> {
    song_like::like(
        &state.redis_conn,
        &state.sql_pool,
        claims.uid(), req.song_id).await?;
    ok!(())
}

#[framed]
async fn unlike(
    claims: Claims,
    state: State<AppState>,
    req: Json<LikeReq>,
) -> WebResult<()> {
    song_like::unlike(
        &state.redis_conn,
        &state.sql_pool,
        req.song_id, claims.uid()).await?;
    ok!(())
}

#[framed]
async fn play() -> WebResult<()> {
    // TODO
    err!("no_impl", "Not implemented")
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagSearchReq {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagSearchResp {
    pub result: Vec<TagItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagItem {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
}

#[framed]
async fn tag_search(
    claims: Claims, // Consider removing auth
    state: State<AppState>,
    req: Query<TagSearchReq>,
) -> WebResult<TagSearchResp> {
    // Validate
    if req.query.is_blank() || (req.query.is_ascii() && req.query.chars().count() < 2) {
        ok!(TagSearchResp { result: vec![] })
    }

    // TODO[opt](tag): Replace with real full-text search
    let result = SongTagDao::search_by_prefix(&state.sql_pool, &req.query).await?
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
    pub id: i64,
}

#[framed]
async fn tag_create(claims: Claims, state: State<AppState>, req: Json<TagCreateReq>) -> WebResult<TagCreateResp> {
    // TODO[feat](song-tag): Need audit procedure
    if req.name.is_empty() || req.name.chars().count() > 10 {
        err!("invalid_name", "Invalid name")
    }

    // TODO[data-racing]: Add a mutex lock for these two operations
    if SongTagDao::get_by_name(&state.sql_pool, req.name.as_str()).await?.is_some() {
        err!("name_exists", "Tag name already exists")
    }

    let id = SongTagDao::insert(
        &state.sql_pool,
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

