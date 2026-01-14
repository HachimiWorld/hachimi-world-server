use crate::db::song::{ISongDao, SongDao};
use crate::db::song_tag::{ISongTagDao, SongTag, SongTagDao};
use crate::db::CrudDao;
use crate::service::song::PublicSongDetail;
use crate::service::{recommend_v2, song, song_like};
use crate::util::IsBlank;
use crate::web::extractors::XRealIP;
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::routes::publish;
use crate::web::state::AppState;
use crate::{err, ok, search};
use async_backtrace::framed;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use chrono::{DateTime, Utc};
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::log::warn;

pub fn router() -> Router<AppState> {
    Router::new()
        // Core operations
        .route("/upload_audio_file", post(publish::upload_audio_file).layer(DefaultBodyLimit::max(20 * 1024 * 1024)))
        .route("/upload_cover_image", post(publish::upload_cover_image).layer(DefaultBodyLimit::max(10 * 1024 * 1024)))
        .route("/delete", post(publish::delete))
        .route("/publish", post(publish::publish))
        .route("/detail", get(detail))
        .route("/detail_by_id", get(detail_by_id))
        .route("/page_by_user", get(page_by_user))
        // Discovery
        .route("/search", get(search))
        .route("/recent_v2", get(recent_v2))
        .route("/hot/weekly", get(hot_weekly))
        .route("/recommend", get(recommend))
        .route("/recommend_anonymous", get(recommend_anonymous))
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
    /// Actually the JMID
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
        &params.id,
    ).await?;
    match data {
        Some(x) => ok!(x),
        None => err!("not_found", "Song not found")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailByIdReq {
    pub id: i64,
}

async fn detail_by_id(
    state: State<AppState>,
    params: Query<DetailByIdReq>,
) -> WebResult<DetailResp> {
    let data = song::get_public_detail_with_cache(
        state.redis_conn.clone(),
        &state.sql_pool,
        &[params.id],
    ).await?.into_iter().next().unwrap();
    match data {
        Some(x) => ok!(x),
        None => err!("not_found", "Song not found")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageByUserReq {
    pub user_id: i64,
    pub page: Option<i64>,
    pub size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageByUserResp {
    pub songs: Vec<DetailResp>,
    pub total: i64,
    pub page: i64,
    pub size: i64,
}

pub struct DeleteReq {
    pub song_id: i64,
}

#[framed]
async fn page_by_user(
    state: State<AppState>,
    req: Query<PageByUserReq>,
) -> WebResult<PageByUserResp> {
    let page = req.page.unwrap_or(0).max(0);
    let size = req.size.unwrap_or(20).min(50);


    // Try to get from the cache first
    if let Some(cached) = page_by_user_cache(state.redis_conn.clone(), req.user_id, page, size).await? {
        ok!(cached)
    }

    // Acquire lock
    let lock = state.red_lock.lock_with_timeout(&format!("user_songs_lock:{}", req.user_id), Duration::from_secs(10)).await?;

    // If the lock is gotten, try to get from the cache again
    if let Some(cached) = page_by_user_cache(state.redis_conn.clone(), req.user_id, page, size).await? {
        ok!(cached)
    }

    let songs = SongDao::page_by_user(&state.sql_pool, req.user_id, page, size).await?;
    let total = SongDao::count_by_user(&state.sql_pool, req.user_id).await?;

    let songs = song::get_public_detail_with_cache(
        state.redis_conn.clone(),
        &state.sql_pool,
        &songs.iter().map(|x| x.id).collect::<Vec<_>>(),
    ).await?;
    let songs: Vec<PublicSongDetail> = songs.into_iter().flatten().collect();

    let resp = PageByUserResp { songs, total, page, size };

    // Cache for 5 minutes
    let _: () = set_page_by_user_cache(state.redis_conn.clone(), req.user_id, page, size, resp.clone()).await?;

    drop(lock);
    ok!(resp)
}

async fn page_by_user_cache(mut redis: ConnectionManager, user_id: i64, page: i64, size: i64) -> anyhow::Result<Option<PageByUserResp>> {
    let cache_key = format!("user_songs:{}:{}:{}", user_id, page, size);
    if let Some(cached) = redis.get::<_, Option<String>>(&cache_key).await? {
        match serde_json::from_str::<PageByUserResp>(&cached) {
            Ok(x) => {
                Ok(Some(x))
            }
            Err(e) => {
                warn!("Failed to parse cache: {:?}", e);
                Ok(None)
            }
        }
    } else {
        Ok(None)
    }
}

async fn set_page_by_user_cache(mut redis: ConnectionManager, user_id: i64, page: i64, size: i64, resp: PageByUserResp) -> anyhow::Result<()> {
    let cache_key = format!("user_songs:{}:{}:{}", user_id, page, size);
    let _: () = redis.set_ex(&cache_key, serde_json::to_string(&resp)?, 300).await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchReq {
    pub q: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub filter: Option<String>,
    /// Since 260114
    pub sort_by: Option<String>
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
    pub uploader_uid: i64,
    pub uploader_name: String,
    pub explicit: Option<bool>,
}

#[framed]
async fn search(
    state: State<AppState>,
    req: Query<SearchReq>,
) -> WebResult<SearchResp> {
    // Validate search
    if req.q.is_blank() && req.filter.is_blank() {
        err!("invalid_query", "Query must not be blank")
    }

    let sort_method = match req.sort_by.as_deref() {
        Some("relevance") | None => None,
        Some("release_time_desc") => Some(search::song::SearchSortMethod::ReleaseTimeDesc),
        Some("release_time_asc") => Some(search::song::SearchSortMethod::ReleaseTimeAsc),
        Some("play_count_desc") => Some(search::song::SearchSortMethod::PlayCountDesc),
        Some("play_count_asc") => Some(search::song::SearchSortMethod::PlayCountAsc),
        Some(other) => {
            err!("invalid_sort_method", "Invalid sort method: {}", other)
        }
    };

    let search_query = search::song::SearchQuery {
        q: req.q.clone(),
        limit: req.limit,
        offset: req.offset,
        filter: req.filter.clone(),
        sort_method,
    };

    let result = search::song::search_songs(state.meilisearch.as_ref(), &search_query).await?;
    let hit_ids: Vec<i64> = result.hits.into_iter().map(|x| x.id).collect();

    let details = song::get_public_detail_with_cache(
        state.redis_conn.clone(),
        &state.sql_pool,
        &hit_ids,
    ).await?
        .into_iter()
        .filter_map(|x| x)
        .map(|song| SearchSongItem {
            id: song.id,
            display_id: song.display_id,
            title: song.title,
            subtitle: song.subtitle,
            description: song.description,
            artist: song.uploader_name.clone(),
            duration_seconds: song.duration_seconds,
            play_count: song.play_count,
            like_count: song.like_count,
            cover_art_url: song.cover_url,
            audio_url: song.audio_url,
            uploader_uid: song.uploader_uid,
            uploader_name: song.uploader_name,
            explicit: song.explicit,
        })
        .collect::<Vec<_>>();

    ok!(SearchResp {
        hits: details,
        query: result.query,
        processing_time_ms: result.processing_time_ms,
        total_hits: result.hits_info.total_hits,
        limit: result.hits_info.limit,
        offset: result.hits_info.offset,
    })
}

/// Since 251102
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentReq {
    pub cursor: Option<DateTime<Utc>>,
    pub limit: Option<i32>,
    pub after: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentResp {
    pub songs: Vec<DetailResp>,
}

#[framed]
async fn recent_v2(
    state: State<AppState>,
    req: Query<RecentReq>,
) -> WebResult<RecentResp> {
    // Validate input
    let limit = req.limit.unwrap_or(50);
    if limit > 50 || limit < 0 {
        err!("invalid_limit", "Limit must be between 0 and 50")
    }
    if limit == 0 {
        ok!(RecentResp {songs: vec![]});
    }

    let after = req.after.unwrap_or(false);

    // ----
    let songs = recommend_v2::get_recent_songs(
        state.red_lock.clone(),
        state.redis_conn.clone(),
        &state.sql_pool,
        req.cursor,
        limit,
        after,
    ).await?;

    ok!(RecentResp {songs})
}

#[derive(Serialize, Deserialize)]
pub struct HotResp {
    pub songs: Vec<DetailResp>,
}

#[framed]
async fn hot_weekly(
    state: State<AppState>
) -> WebResult<HotResp> {
    let songs = recommend_v2::get_hot_songs(&state.redis_conn, &state.sql_pool, 7, 50).await?;
    ok!(HotResp {songs})
}

#[derive(Serialize, Deserialize)]
pub struct RecommendResp {
    pub songs: Vec<DetailResp>,
}

async fn recommend(
    claims: Claims,
    state: State<AppState>,
) -> WebResult<RecommendResp> {
    let recommend = recommend_v2::get_recommend(claims.uid(), state.red_lock.clone(), state.redis_conn.clone(), &state.sql_pool).await?;
    let resp = RecommendResp { songs: recommend };
    ok!(resp)
}

async fn recommend_anonymous(
    ip: XRealIP,
    state: State<AppState>,
) -> WebResult<RecommendResp> {
    let recommend = recommend_v2::get_recommend_anonymous(&ip.0, state.red_lock.clone(), state.redis_conn.clone(), &state.sql_pool).await?;
    let resp = RecommendResp { songs: recommend };
    ok!(resp)
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
    _claims: Claims, // Consider removing auth
    state: State<AppState>,
    req: Query<TagSearchReq>,
) -> WebResult<TagSearchResp> {
    // Validate
    if req.query.is_blank() {
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
        },
    ).await?;

    ok!(TagCreateResp { id })
}