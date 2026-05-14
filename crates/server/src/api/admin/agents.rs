//! `GET /admin/agents`, `GET /admin/libraries/{id}/agents`,
//! `PUT /admin/libraries/{id}/agents`.
//!
//! The agent registry is built at startup from the metadata clients we
//! actually have on `AppState`. An agent only appears in `/admin/agents`
//! if its client is constructed — TMDB requires a token, TVMaze is
//! always available.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use chimpflix_library::{AgentInfo, LibraryAgent, NewAuditEntry, queries};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct AgentsListResponse {
    pub agents: Vec<AgentInfo>,
}

#[derive(Debug, Serialize)]
pub struct LibraryAgentsResponse {
    pub agents: Vec<LibraryAgent>,
}

#[derive(Debug, Deserialize)]
pub struct SetLibraryAgentsInput {
    pub agents: Vec<LibraryAgent>,
}

pub async fn list_available(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<AgentsListResponse>, ApiError> {
    Ok(Json(AgentsListResponse {
        agents: build_registry(&state),
    }))
}

pub async fn get_for_library(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
) -> Result<Json<LibraryAgentsResponse>, ApiError> {
    if queries::get_library(&state.pool, library_id)
        .await
        .map_err(ApiError::Internal)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    let agents = queries::list_library_agents(&state.pool, library_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(LibraryAgentsResponse { agents }))
}

pub async fn set_for_library(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(library_id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<SetLibraryAgentsInput>,
) -> Result<Json<LibraryAgentsResponse>, ApiError> {
    if queries::get_library(&state.pool, library_id)
        .await
        .map_err(ApiError::Internal)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }

    // Reject duplicate agent_name entries and unknown agents.
    let registry = build_registry(&state);
    let known: std::collections::HashSet<&str> =
        registry.iter().map(|a| a.name.as_str()).collect();
    let mut seen = std::collections::HashSet::new();
    for a in &input.agents {
        if !seen.insert(a.agent_name.as_str()) {
            return Err(ApiError::validation(format!(
                "agent `{}` listed more than once",
                a.agent_name
            )));
        }
        if !known.contains(a.agent_name.as_str()) {
            return Err(ApiError::validation(format!(
                "unknown agent `{}`",
                a.agent_name
            )));
        }
    }

    let agents = queries::set_library_agents(&state.pool, library_id, &input.agents)
        .await
        .map_err(ApiError::Internal)?;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "library.agents.update".into(),
            target_kind: Some("library".into()),
            target_id: Some(library_id.to_string()),
            payload_json: serde_json::to_string(&input.agents).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;

    Ok(Json(LibraryAgentsResponse { agents }))
}

fn build_registry(state: &AppState) -> Vec<AgentInfo> {
    // The set of agents we ship today. `configured` reflects whether the
    // client is actually constructed on AppState (TMDB requires a token);
    // disabled agents can still be listed for the owner to see, but won't
    // produce metadata when run.
    vec![
        AgentInfo {
            name: "tmdb".into(),
            display_name: "The Movie Database".into(),
            supported_kinds: vec!["movie".into(), "show".into()],
            configured: state.tmdb.is_some(),
        },
        AgentInfo {
            name: "tvmaze".into(),
            display_name: "TVmaze".into(),
            supported_kinds: vec!["show".into()],
            configured: state.tvmaze.is_some(),
        },
    ]
}
