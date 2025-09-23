use crate::db::song::{ISongDao, SongDao, SongPlay};
use crate::service::song::{get_public_detail_with_cache, PublicSongDetail};
use crate::web::extractors::XRealIP;
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{err, ok};
use anyhow::Context;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
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
    ok!(())
}

async fn touch_anonymous(
    ip: XRealIP,
    mut state: State<AppState>,
    req: Json<TouchReq>
) -> WebResult<()>{
    // Convert to anonymous uid
    let anonymous_uid = convert_ip_to_anonymous_uid(&ip.0)?;

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
    ok!(())
}

fn convert_ip_to_anonymous_uid(ip: &str) -> anyhow::Result<i64> {
    // 23.224.125.1
    // to 23224125001
    let parts = ip.split('.').take(4)
        .map(|x| x.parse::<i64>())
        .map(|x| x.map(|x| format!("{:03}", x)))
        .collect::<Result<Vec<_>, _>>()
        .map(|x| x.join(""))
        .with_context(|| format!("Invalid IP address: {ip}"))?;
    parts.parse::<i64>()
        .context(format!("Invalid IP address: {ip}"))
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

#[cfg(test)]
mod tests {
    use crate::web::routes::play_history::convert_ip_to_anonymous_uid;

    #[test]
    fn test_convert_ip_to_anonymous_uid() {
        assert_eq!(23224125001, convert_ip_to_anonymous_uid("23.224.125.1").unwrap());
    }
}