use crate::db::creator::CreatorDao;
use crate::db::song::{ISongDao, SongDao};
use crate::db::song_publishing_review;
use crate::db::song_publishing_review::{ISongPublishingReviewDao, SongPublishingReviewDao};
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::state::AppState;
use crate::{common, err, ok};
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidCheckPReq {
    pub jmid_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidCheckPResp {
    pub result: bool,
}

/// Check if the jmid prefix part is not used by anyone
pub async fn jmid_check_prefix(
    _: Claims,
    state: State<AppState>,
    req: Query<JmidCheckPReq>,
) -> WebResult<JmidCheckPResp> {
    let r = CreatorDao::get_by_jmid_prefix(&state.sql_pool, &req.jmid_prefix).await?;
    ok!(JmidCheckPResp {result: r.is_none()})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidCheckReq {
    pub jmid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidCheckResp {
    pub result: bool,
}

/// Check if the full jmid is available
pub async fn jmid_check(
    _: Claims,
    state: State<AppState>,
    req: Query<JmidCheckReq>,
) -> WebResult<JmidCheckResp> {
    let r = check_jmid_available(&state.sql_pool, &req.jmid).await?;
    ok!(JmidCheckResp {result: r})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidMineResp {
    pub jmid_prefix: Option<String>,
}

pub async fn jmid_mine(
    claims: Claims,
    state: State<AppState>,
) -> WebResult<JmidMineResp> {
    let creator = CreatorDao::get_by_user_id(&state.sql_pool, claims.uid()).await?;
    let jmid_prefix = creator.map(|x| x.jmid_prefix);
    ok!(JmidMineResp {jmid_prefix})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmidGetNextResp {
    pub jmid: String,
}

/// Get the next available jmid for this creator.
/// Only available for a creator who had already specified a jm-code
/// # Errors
/// - `jmid_prefix_not_specified`
/// - `jmid_prefix_inactive`
pub async fn jmid_get_next(
    claims: Claims,
    State(state): State<AppState>,
) -> WebResult<JmidGetNextResp> {
    let creator = CreatorDao::get_by_user_id(&state.sql_pool, claims.uid()).await?
        .ok_or_else(|| common!("jmid_prefix_not_specified", "You have not specified a jmid prefix yet"))?;

    if !creator.active {
        err!("jmid_prefix_inactive", "Your jmid prefix is not active yet, please wait for processing")
    }

    // Count all songs of the creator and add the pending PRs
    let published_songs = SongDao::count_by_user(&state.sql_pool, claims.uid()).await?;
    let pending_prs = SongPublishingReviewDao::count_by_user_and_status(&state.sql_pool, claims.uid(), song_publishing_review::STATUS_PENDING).await?;

    let next_no = published_songs + pending_prs + 1;
    let jmid = format!("{}-{:03}", creator.jmid_prefix, next_no);

    ok!(JmidGetNextResp {jmid})
}

/// Check whether the `jmid` is available (not used nor locked by other pending SRs).
pub async fn check_jmid_available(
    sql: &PgPool,
    jmid: &str,
) -> anyhow::Result<bool> {
    // Check the songs
    let song = SongDao::get_by_display_id(sql, &jmid).await?;
    if song.is_some() {
        return Ok(false);
    }

    // Check the pending SRs
    // guarantee: If a SR is approved or rejected, the song's display id will be changed to the latest one. So we do not need to check it.
    let prs: Vec<_> = SongPublishingReviewDao::list_by_jmid(sql, jmid).await?;
    let has_pending_prs = prs.iter().any(|x| {
        x.song_display_id == jmid && x.status == 0
    });
    Ok(!has_pending_prs)
}

pub fn parse_jmid(input: &str) -> Option<(&str, &str)> {
    let regex = regex::Regex::new(r"^JM-([A-Z]{3,4})-?(\d{3})$").ok()?;
    let captures = regex.captures(input)?;
    Some((
        captures.get(1)?.as_str(),
        captures.get(2)?.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use crate::web::routes::publish::parse_jmid;

    #[test]
    fn test_parse_jmid() {
        assert_eq!(parse_jmid("JM-ABC-123"), Some(("ABC", "123")));
        assert_eq!(parse_jmid("JM-ABCD-001"), Some(("ABCD", "001")));
        assert_eq!(parse_jmid("JM-ABCD-1"), None);
        assert_eq!(parse_jmid("JM-ABCD-ABC"), None);
        assert_eq!(parse_jmid("JM-A-001"), None);
        assert_eq!(parse_jmid("ABC-123"), None);
        assert_eq!(parse_jmid("ABCD123"), None);
        assert_eq!(parse_jmid("JM-abc-123"), None);
    }
}