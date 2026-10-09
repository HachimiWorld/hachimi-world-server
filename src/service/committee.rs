//! The committee: members appointed by contributors can read the report queue; only contributors
//! decide on reports.

use crate::db::committee_member::{CommitteeMember, CommitteeMemberDao};
use crate::db::user::UserDao;
use crate::db::CrudDao;
use crate::service::contributor;
use crate::service::errors::{ServiceError, ServiceResult};
use crate::web::state::AppState;
use anyhow::anyhow;
use chrono::{SubsecRound, Utc};

pub struct Access {
    /// Committee members and contributors.
    pub can_view: bool,
    /// Contributors.
    pub can_resolve: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CommitteeError {
    #[error("user not found")]
    UserNotFound,
    #[error("already a committee member")]
    AlreadyMember,
    #[error("not a committee member")]
    NotMember,
}

pub async fn access(state: &AppState, uid: i64) -> anyhow::Result<Access> {
    let is_contributor = contributor::check_contributor(&state.config, state.redis_conn.clone(), &state.red_lock, &state.sql_pool, uid).await?;
    let is_member = CommitteeMemberDao::get(&state.sql_pool, uid).await?.is_some();
    Ok(Access { can_view: is_contributor || is_member, can_resolve: is_contributor })
}

pub async fn appoint(state: &AppState, operator_uid: i64, uid: i64) -> ServiceResult<(), CommitteeError> {
    if UserDao::get_by_id(&state.sql_pool, uid).await?.is_none() {
        return Err(ServiceError::BusinessError(CommitteeError::UserNotFound));
    }
    let _lock = state.red_lock.lock_with_timeout(&format!("committee:member:{uid}"), std::time::Duration::from_secs(10)).await?
        .ok_or_else(|| anyhow!("Can't get the committee member lock"))?;
    if CommitteeMemberDao::get(&state.sql_pool, uid).await?.is_some() {
        return Err(ServiceError::BusinessError(CommitteeError::AlreadyMember));
    }
    CommitteeMemberDao::insert(&state.sql_pool, &CommitteeMember {
        uid,
        appointed_by_uid: operator_uid,
        create_time: Utc::now().trunc_subsecs(6),
    }).await?;
    Ok(())
}

pub async fn revoke(state: &AppState, uid: i64) -> ServiceResult<(), CommitteeError> {
    if !CommitteeMemberDao::delete(&state.sql_pool, uid).await? {
        return Err(ServiceError::BusinessError(CommitteeError::NotMember));
    }
    Ok(())
}
