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
    use regex::Regex;

    // IPv4 pattern
    let ipv4_re = Regex::new(r"^(\d{1,3}\.){3}\d{1,3}$").unwrap();
    // IPv6 pattern (simplified, handles basic format)
    let ipv6_re = Regex::new(r"^([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}$").unwrap();

    if ipv4_re.is_match(ip) {
        // Convert IPv4 to number
        let parts: Vec<i64> = ip.split('.')
            .map(|x| x.parse::<i64>())
            .collect::<Result<Vec<_>, _>>()
            .with_context(|| format!("Invalid IPv4 address: {ip}"))?;

        if parts.iter().any(|&x| x > 255) {
            anyhow::bail!("Invalid IPv4 address: {ip}");
        }

        Ok(parts[0] * 1_000_000_000 + parts[1] * 1_000_000 + parts[2] * 1_000 + parts[3])
    } else if ipv6_re.is_match(ip) {
        // Convert IPv6 to number by taking first 4 segments
        let parts: Vec<i64> = ip.split(':')
            .take(4)
            .map(|x| i64::from_str_radix(x, 16))
            .collect::<Result<Vec<_>, _>>()
            .with_context(|| format!("Invalid IPv6 address: {ip}"))?;

        Ok(parts[0] * 1_000_000_000 + parts[1] * 1_000_000 + parts[2] * 1_000 + parts[3])
    } else {
        anyhow::bail!("Unsupported IP address format: {ip}")
    }
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
    use super::*;

    #[test]
    fn test_convert_ip_to_anonymous_uid() {
        assert_eq!(23224125001, convert_ip_to_anonymous_uid("23.224.125.1").unwrap());
        assert_eq!(192168001254, convert_ip_to_anonymous_uid("192.168.1.254").unwrap());
        assert_eq!(4660086001929, convert_ip_to_anonymous_uid("1234:0056:0000:0789:1234:5678:9abc:def0").unwrap());
        assert!(convert_ip_to_anonymous_uid("256.1.2.3").is_err());
        assert!(convert_ip_to_anonymous_uid("invalid").is_err());
    }
}
