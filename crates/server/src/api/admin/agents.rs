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
use chimpflix_library::{AgentCapabilitiesDto, AgentInfo, LibraryAgent, NewAuditEntry, queries};
use chimpflix_metadata::agents::{static_capabilities_for, static_limitations_for};
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
        agents: build_registry(&state).await,
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
    let registry = build_registry(&state).await;
    let known: std::collections::HashSet<&str> = registry.iter().map(|a| a.name.as_str()).collect();
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

async fn build_registry(state: &AppState) -> Vec<AgentInfo> {
    // The set of agents we ship today. `configured` reflects whether the
    // client is actually constructed on AppState (TMDB and TVDB require
    // tokens); disabled agents can still be listed for the owner to see,
    // but won't produce metadata when run.
    //
    // `participates_in_chain` distinguishes true metadata agents that
    // the scanner dispatch loop in `crates/library/src/scanner.rs` knows
    // how to invoke (tmdb / tvdb / tvmaze / anilist) from non-metadata
    // providers exposed only for credential-status visibility
    // (opensubtitles, trakt, omdb). The latter are triggered via their
    // own dedicated paths — OMDb via the `fetch_external_ratings`
    // background job, OpenSubtitles via the subtitle-search code path,
    // Trakt via /settings/integrations sync. Before this flag existed,
    // operators could add a non-chain agent (e.g. OMDb) to a library's
    // priority list and it would silently no-op, suggesting it was
    // doing work when it wasn't.
    let tmdb_configured = state.tmdb.read().await.is_some();
    let tvdb_configured = state.tvdb.read().await.is_some();
    let anilist_configured = state.anilist.read().await.is_some();
    let opensubtitles_configured = state.opensubtitles.read().await.is_some();
    let trakt_configured = state.trakt.read().await.is_some();
    let omdb_configured = state.omdb.read().await.is_some();
    // Helper to look up the static capability + limitations strings
    // by agent name. Returns empty for non-chain agents (opensubtitles,
    // trakt) — those don't go through the MetadataAgent trait.
    let caps = |name: &str| AgentCapabilitiesDto::from(static_capabilities_for(name));
    let limits = |name: &str| -> Vec<String> {
        static_limitations_for(name)
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    };
    vec![
        AgentInfo {
            name: "tmdb".into(),
            display_name: "The Movie Database".into(),
            supported_kinds: vec!["movie".into(), "show".into()],
            configured: tmdb_configured,
            participates_in_chain: true,
            capabilities: caps("tmdb"),
            limitations: limits("tmdb"),
        },
        AgentInfo {
            name: "tvdb".into(),
            display_name: "TheTVDB".into(),
            supported_kinds: vec!["movie".into(), "show".into()],
            configured: tvdb_configured,
            participates_in_chain: true,
            capabilities: caps("tvdb"),
            limitations: limits("tvdb"),
        },
        AgentInfo {
            name: "tvmaze".into(),
            display_name: "TVmaze".into(),
            supported_kinds: vec!["show".into()],
            configured: state.tvmaze.is_some(),
            participates_in_chain: true,
            capabilities: caps("tvmaze"),
            limitations: limits("tvmaze"),
        },
        AgentInfo {
            name: "anilist".into(),
            display_name: "AniList".into(),
            // AniList is anime-only; agents.rs uses `supported_kinds` to
            // gate which agents the per-library picker offers, so we
            // tag it `show` (anime resolves to ItemKind::Show) and rely
            // on the seed defaults to enable it only for anime libraries.
            supported_kinds: vec!["show".into()],
            configured: anilist_configured,
            participates_in_chain: true,
            capabilities: caps("anilist"),
            limitations: limits("anilist"),
        },
        AgentInfo {
            name: "opensubtitles".into(),
            display_name: "OpenSubtitles".into(),
            // Subtitle agents apply to anything playable; expose both
            // kinds so the per-library picker offers it everywhere.
            supported_kinds: vec!["movie".into(), "show".into()],
            configured: opensubtitles_configured,
            participates_in_chain: false,
            capabilities: AgentCapabilitiesDto::default(),
            limitations: vec!["Subtitle search only; not a metadata agent.".to_string()],
        },
        AgentInfo {
            name: "trakt".into(),
            display_name: "Trakt".into(),
            // Trakt is a sync target, not a per-library metadata agent
            // — but listing it here keeps the configured-status visible
            // alongside the metadata providers. Users link/unlink
            // individually from /settings/integrations.
            supported_kinds: vec!["movie".into(), "show".into()],
            configured: trakt_configured,
            participates_in_chain: false,
            capabilities: AgentCapabilitiesDto::default(),
            limitations: vec![
                "Two-way watch-state sync; doesn't supply metadata or artwork.".to_string(),
            ],
        },
        AgentInfo {
            name: "omdb".into(),
            display_name: "OMDb".into(),
            // Slice 7 promoted OMDb to a full chain participant.
            // It can supply movie/show/episode metadata as well as
            // ratings — though the 1k req/day cap means operators
            // should place it late in the chain for large libraries.
            supported_kinds: vec!["movie".into(), "show".into()],
            configured: omdb_configured,
            participates_in_chain: true,
            capabilities: caps("omdb"),
            limitations: limits("omdb"),
        },
    ]
}
