use crate::db::song::{ISongDao, SongDao, SongPlay};
use crate::service::song::{get_public_detail_with_cache, PublicSongDetail};
use crate::web::extractors::XRealIP;
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{err, ok, util};
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use metrics::gauge;
use redis::{AsyncCommands, ExistenceCheck, SetExpiry, SetOptions};
use redis::aio::ConnectionManager;
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/cursor", get(cursor))
        .route("/touch", post(touch))
        .route("/touch_anonymous", post(touch_anonymous))
        .route("/delete", post(delete))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorReq {
    pub cursor: Option<DateTime<Utc>>,
    pub size: usize
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorResp {
    pub list: Vec<PlayHistoryItem>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayHistoryItem {
    pub id: i64,
    pub song_info: PublicSongDetail,
    pub play_time: DateTime<Utc>
}

async fn cursor(
    claims: Claims,
    state: State<AppState>,
    req: Query<CursorReq>
) -> WebResult<CursorResp> {
    if req.size > 64 {
        err!("size_exceeded", "Page size must be less than 64")
    }
    let history = SongDao::cursor_plays(&state.sql_pool, claims.uid(), req.cursor.unwrap_or_else(|| Utc::now()), req.size).await?;
    let mut result = Vec::new();
    for x in history {
        if let Some(detail) = get_public_detail_with_cache(
            state.redis_conn.clone(),
            &state.sql_pool,
            x.song_id
        ).await? {
            result.push(PlayHistoryItem {
                id: x.id,
                song_info: detail,
                play_time: x.create_time,
            })
        }
    }

    ok!(CursorResp { list: result })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchReq {
    pub song_id: i64
}

async fn touch(
    claims: Claims,
    mut state: State<AppState>,
    req: Json<TouchReq>
) -> WebResult<()> {
    if cooldown(claims.uid(), req.song_id, &mut state.redis_conn).await? {
        err!("cooldown", "Please wait 60 seconds before touching again");
    }

    let data = SongPlay {
        id: 0,
        song_id: req.song_id,
        user_id: Some(claims.uid()),
        anonymous_uid: None,
        create_time: Utc::now(),
    };
    SongDao::insert_plays(&state.sql_pool, &[data]).await?;

    let dau_key = format!("dau:hll:{}", Utc::now().date_naive().to_string());

    let r: bool = state.redis_conn.pfadd(&dau_key, claims.uid()).await?;
    if r {
        let dau: i64 = state.redis_conn.pfcount(dau_key).await?;
        gauge!("daily_active_user").set(dau as f64);
    }

    ok!(())
}

async fn touch_anonymous(
    ip: XRealIP,
    mut state: State<AppState>,
    req: Json<TouchReq>
) -> WebResult<()>{
    // Convert to anonymous uid
    let anonymous_uid = util::convert_ip_to_anonymous_uid(&ip.0)?;

    if cooldown(anonymous_uid, req.song_id, &mut state.redis_conn).await? {
        err!("cooldown", "Please wait 60 seconds before touching again");
    }
    let data = SongPlay {
        id: 0,
        song_id: req.song_id,
        user_id: None,
        anonymous_uid: Some(anonymous_uid),
        create_time: Utc::now(),
    };
    SongDao::insert_plays(&state.sql_pool, &[data]).await?;

    let daau = format!("dau_anonymous:hll:{}", Utc::now().date_naive().to_string());

    let r: bool = state.redis_conn.pfadd(&daau, &ip.0).await?;
    if r {
        let dau: i64 = state.redis_conn.pfcount(daau).await?;
        gauge!("daily_active_anonymous_user").set(dau as f64);
    }
    ok!(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteReq {
    pub history_id: i64,
}

async fn delete(
    claims: Claims,
    state: State<AppState>,
    req: Json<DeleteReq>
) -> WebResult<()> {
    SongDao::delete_play(&state.sql_pool, claims.uid(), req.history_id).await?;
    ok!(())
}

async fn cooldown(
    user_id: i64,
    song_id: i64,
    redis: &mut ConnectionManager
) -> anyhow::Result<bool> {
    let cooldown_key = format!("play:touch_cooldown:{}:{}", user_id, song_id);
    let cooldown_absent: bool = redis.set_options(
        cooldown_key, 0,
        SetOptions::default().conditional_set(ExistenceCheck::NX)
            .with_expiration(SetExpiry::EX(60)) // CD for 60 secs
    ).await?;
    Ok(!cooldown_absent)
}