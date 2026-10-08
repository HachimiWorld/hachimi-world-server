use crate::db::committee_member::CommitteeMemberDao;
use crate::service::committee::{self, CommitteeError};
use crate::service::errors::ServiceError;
use crate::web::jwt::Claims;
use crate::web::result::{CommonError, WebError, WebResult};
use crate::web::routes::report::{load_users, UserBrief};
use crate::web::state::AppState;
use crate::{common, err, ok};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// @since 261008
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me", get(me))
        .route("/members", get(members))
        .route("/appoint", post(appoint))
        .route("/revoke", post(revoke))
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct MeResp {
    /// Can read the report queue: committee members and contributors.
    pub can_view: bool,
    /// Can decide on reports and appoint members: contributors.
    pub can_resolve: bool,
}

async fn me(claims: Claims, state: State<AppState>) -> WebResult<MeResp> {
    let access = committee::access(&state, claims.uid()).await?;
    ok!(MeResp { can_view: access.can_view, can_resolve: access.can_resolve })
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct MembersResp {
    /// Oldest appointment first.
    pub members: Vec<MemberItem>,
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct MemberItem {
    pub user: UserBrief,
    pub appointed_by: Option<UserBrief>,
    pub appoint_time: DateTime<Utc>,
}

async fn members(claims: Claims, state: State<AppState>) -> WebResult<MembersResp> {
    if !committee::access(&state, claims.uid()).await?.can_view {
        err!("permission_denied", "Only the committee and contributors can view members")
    }
    let members = CommitteeMemberDao::list_all(&state.sql_pool).await?;
    let uids: Vec<i64> = members.iter().flat_map(|x| [x.uid, x.appointed_by_uid]).collect();
    let users = load_users(&state.sql_pool, &uids).await?;
    let members = members.into_iter()
        .filter_map(|x| Some(MemberItem {
            user: users.get(&x.uid)?.clone(),
            appointed_by: users.get(&x.appointed_by_uid).cloned(),
            appoint_time: x.create_time,
        }))
        .collect();
    ok!(MembersResp { members })
}

/// @since 261008
#[derive(Debug, Serialize, Deserialize)]
pub struct MemberReq {
    pub uid: i64,
}

async fn appoint(claims: Claims, state: State<AppState>, req: Json<MemberReq>) -> WebResult<()> {
    require_contributor(&state, claims.uid()).await?;
    committee::appoint(&state, claims.uid(), req.uid).await?;
    ok!(())
}

async fn revoke(claims: Claims, state: State<AppState>, req: Json<MemberReq>) -> WebResult<()> {
    require_contributor(&state, claims.uid()).await?;
    committee::revoke(&state, req.uid).await?;
    ok!(())
}

async fn require_contributor(state: &AppState, uid: i64) -> Result<(), WebError<CommonError>> {
    if committee::access(state, uid).await?.can_resolve {
        Ok(())
    } else {
        Err(common!("permission_denied", "Only contributors can manage the committee"))
    }
}

impl From<ServiceError<CommitteeError>> for WebError<CommonError> {
    fn from(err: ServiceError<CommitteeError>) -> Self {
        match err {
            ServiceError::BusinessError(e) => {
                let code = match e {
                    CommitteeError::UserNotFound => "user_not_found",
                    CommitteeError::AlreadyMember => "already_member",
                    CommitteeError::NotMember => "not_member",
                };
                common!(code, "{}", e)
            }
            ServiceError::Other(e) => WebError::Internal(e),
        }
    }
}
