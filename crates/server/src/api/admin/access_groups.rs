//! Admin endpoints for named access groups.
//!
//! Groups are an additive bulk-assignment layer on top of the existing
//! per-user `library_access` rows. Effective access for any user is the
//! UNION of their direct grants and the libraries of every group they
//! belong to — see [`queries::user_library_filter`] for the resolution
//! rules.
//!
//! Every mutation audit-logs through the usual admin helper so the
//! trail covers group create/update/delete + member/library set
//! changes.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::USER_AGENT;
use chimpflix_library::{
    AccessGroup, AccessGroupDetail, AccessGroupUpdate, NewAccessGroup, NewAuditEntry, queries,
};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

const MAX_GROUP_NAME_LEN: usize = 64;
const MAX_GROUP_DESCRIPTION_LEN: usize = 280;
const MAX_GROUP_MEMBERS: usize = 1024;
const MAX_GROUP_LIBRARIES: usize = 256;

#[derive(Debug, Serialize)]
pub struct GroupsListResponse {
    pub groups: Vec<AccessGroup>,
}

pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<GroupsListResponse>, ApiError> {
    let groups = queries::list_access_groups(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(GroupsListResponse { groups }))
}

pub async fn create(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<NewAccessGroup>,
) -> Result<(StatusCode, Json<AccessGroup>), ApiError> {
    validate_name(&input.name)?;
    if let Some(ref d) = input.description {
        if d.len() > MAX_GROUP_DESCRIPTION_LEN {
            return Err(ApiError::validation(
                "description must be at most 280 characters",
            ));
        }
    }
    let group = queries::create_access_group(&state.pool, input)
        .await
        .map_err(|e| {
            let msg = format!("{e:#}");
            if msg.contains("UNIQUE constraint failed") {
                ApiError::Conflict("a group with that name already exists".into())
            } else {
                ApiError::Internal(e)
            }
        })?;
    audit_change(
        &state,
        actor.id,
        "access_group.create",
        group.id,
        Some(format!(r#"{{"name":{}}}"#, json_str(&group.name))),
        &headers,
    )
    .await;
    Ok((StatusCode::CREATED, Json(group)))
}

pub async fn get_one(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(id): Path<i64>,
) -> Result<Json<AccessGroupDetail>, ApiError> {
    queries::get_access_group_detail(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

pub async fn update(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<AccessGroupUpdate>,
) -> Result<Json<AccessGroup>, ApiError> {
    if let Some(ref name) = input.name {
        validate_name(name)?;
    }
    if let Some(Some(ref d)) = input.description {
        if d.len() > MAX_GROUP_DESCRIPTION_LEN {
            return Err(ApiError::validation(
                "description must be at most 280 characters",
            ));
        }
    }
    let updated = queries::update_access_group(&state.pool, id, input)
        .await
        .map_err(|e| {
            let msg = format!("{e:#}");
            if msg.contains("UNIQUE constraint failed") {
                ApiError::Conflict("a group with that name already exists".into())
            } else {
                ApiError::Internal(e)
            }
        })?
        .ok_or(ApiError::NotFound)?;
    audit_change(
        &state,
        actor.id,
        "access_group.update",
        id,
        Some(format!(r#"{{"name":{}}}"#, json_str(&updated.name))),
        &headers,
    )
    .await;
    Ok(Json(updated))
}

pub async fn delete(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let removed = queries::delete_access_group(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    audit_change(&state, actor.id, "access_group.delete", id, None, &headers).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct SetLibrariesRequest {
    pub library_ids: Vec<i64>,
}

pub async fn set_libraries(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<SetLibrariesRequest>,
) -> Result<StatusCode, ApiError> {
    if input.library_ids.len() > MAX_GROUP_LIBRARIES {
        return Err(ApiError::validation(format!(
            "a group can bind at most {MAX_GROUP_LIBRARIES} libraries"
        )));
    }
    // Ensure the group exists; SQL would happily insert orphan rows
    // otherwise (FKs would catch it, but the error wouldn't be a 404).
    if queries::get_access_group_detail(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    queries::set_access_group_libraries(&state.pool, id, &input.library_ids)
        .await
        .map_err(ApiError::Internal)?;
    audit_change(
        &state,
        actor.id,
        "access_group.libraries.update",
        id,
        Some(format!(r#"{{"count":{}}}"#, input.library_ids.len())),
        &headers,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct SetMembersRequest {
    pub user_ids: Vec<i64>,
}

pub async fn set_members(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(input): Json<SetMembersRequest>,
) -> Result<StatusCode, ApiError> {
    if input.user_ids.len() > MAX_GROUP_MEMBERS {
        return Err(ApiError::validation(format!(
            "a group can hold at most {MAX_GROUP_MEMBERS} members"
        )));
    }
    if queries::get_access_group_detail(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    queries::set_access_group_members(&state.pool, id, &input.user_ids)
        .await
        .map_err(ApiError::Internal)?;
    audit_change(
        &state,
        actor.id,
        "access_group.members.update",
        id,
        Some(format!(r#"{{"count":{}}}"#, input.user_ids.len())),
        &headers,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
pub struct UserGroupsResponse {
    pub group_ids: Vec<i64>,
}

pub async fn get_user_groups(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(user_id): Path<i64>,
) -> Result<Json<UserGroupsResponse>, ApiError> {
    let group_ids = queries::list_user_group_ids(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(UserGroupsResponse { group_ids }))
}

#[derive(Debug, Deserialize)]
pub struct SetUserGroupsRequest {
    pub group_ids: Vec<i64>,
}

pub async fn set_user_groups(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(input): Json<SetUserGroupsRequest>,
) -> Result<StatusCode, ApiError> {
    if input.group_ids.len() > 64 {
        return Err(ApiError::validation(
            "a user can belong to at most 64 groups",
        ));
    }
    queries::set_user_groups(&state.pool, user_id, &input.group_ids)
        .await
        .map_err(ApiError::Internal)?;
    audit_change(
        &state,
        actor.id,
        "user.groups.update",
        user_id,
        Some(format!(r#"{{"count":{}}}"#, input.group_ids.len())),
        &headers,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_name(name: &str) -> Result<(), ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::validation("name must not be empty"));
    }
    if trimmed.len() > MAX_GROUP_NAME_LEN {
        return Err(ApiError::validation(format!(
            "name must be at most {MAX_GROUP_NAME_LEN} characters"
        )));
    }
    Ok(())
}

async fn audit_change(
    state: &AppState,
    actor_id: i64,
    action: &str,
    target_id: i64,
    payload_json: Option<String>,
    headers: &HeaderMap,
) {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        state,
        NewAuditEntry {
            actor_user_id: Some(actor_id),
            action: action.to_string(),
            target_kind: Some("access_group".to_string()),
            target_id: Some(target_id.to_string()),
            payload_json,
            ip: None,
            user_agent,
        },
    )
    .await;
}

/// Minimal JSON string escaper for audit payloads. We don't want to
/// pull serde_json::to_string in here for a single value — and the
/// audit row is fine with hand-built JSON since the only field is a
/// validated name (no embedded newlines/control chars to worry about).
fn json_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
