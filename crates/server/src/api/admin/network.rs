//! Network admin surface: focused view over the network-related
//! `server_settings` fields plus a reachability self-check.
//!
//! GET / PATCH delegate to the same canonical settings storage Phase-1
//! built; the reachability endpoint performs a one-shot HTTP HEAD against
//! `<public_url>/api/v1/health` from the server's own egress.

use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use chimpflix_library::{NewAuditEntry, queries};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::net;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct NetworkResponse {
    pub public_url: Option<String>,
    pub cors_origins: Vec<String>,
    pub secure_connections: String,
    // ── Phase 32 additions ──
    pub transcoder_reaper_idle_threshold_ms: i64,
    pub max_remote_streams_per_user: i64,
    pub lan_networks: String,
    pub auth_bypass_cidrs: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NetworkUpdate {
    /// Send `null` to clear `public_url`; omit the key to leave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_url: Option<Option<String>>,
    /// Array of allowed origins (no trailing slash). Pass the empty array
    /// to clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cors_origins: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure_connections: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcoder_reaper_idle_threshold_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_remote_streams_per_user: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan_networks: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_bypass_cidrs: Option<String>,
}

fn response_from(s: chimpflix_library::ServerSettings) -> NetworkResponse {
    let cors_origins: Vec<String> = serde_json::from_str(&s.cors_origins).unwrap_or_default();
    NetworkResponse {
        public_url: s.public_url,
        cors_origins,
        secure_connections: s.secure_connections,
        transcoder_reaper_idle_threshold_ms: s.transcoder_reaper_idle_threshold_ms,
        max_remote_streams_per_user: s.max_remote_streams_per_user,
        lan_networks: s.lan_networks,
        auth_bypass_cidrs: s.auth_bypass_cidrs,
    }
}

pub async fn get(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<NetworkResponse>, ApiError> {
    let s = state.settings.read().await.clone();
    Ok(Json(response_from(s)))
}

pub async fn patch(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<NetworkUpdate>,
) -> Result<Json<NetworkResponse>, ApiError> {
    if let Some(ref mode) = input.secure_connections {
        if !matches!(mode.as_str(), "required" | "preferred" | "disabled") {
            return Err(ApiError::validation(
                "secure_connections must be one of: required, preferred, disabled",
            ));
        }
    }
    if let Some(Some(ref url)) = input.public_url {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ApiError::validation(
                "public_url must start with http:// or https://",
            ));
        }
    }
    if let Some(n) = input.transcoder_reaper_idle_threshold_ms {
        // 5s floor — anything lower and the 60s client keepalive plus
        // 15s reaper interval would race-kill healthy sessions. 1h
        // ceiling so a typo can't strand sessions for a day.
        if !(5_000..=3_600_000).contains(&n) {
            return Err(ApiError::validation(
                "transcoder_reaper_idle_threshold_ms must be between 5000 and 3600000",
            ));
        }
    }
    if let Some(n) = input.max_remote_streams_per_user {
        if !(0..=64).contains(&n) {
            return Err(ApiError::validation(
                "max_remote_streams_per_user must be between 0 (unlimited) and 64",
            ));
        }
    }
    if let Some(ref raw) = input.lan_networks {
        net::validate_cidr_list(raw)
            .map_err(|e| ApiError::validation(format!("lan_networks: {e}")))?;
    }
    if let Some(ref raw) = input.auth_bypass_cidrs {
        net::validate_cidr_list(raw)
            .map_err(|e| ApiError::validation(format!("auth_bypass_cidrs: {e}")))?;
    }

    let cors_serialized = input
        .cors_origins
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string()));

    let patch = chimpflix_library::ServerSettingsUpdate {
        public_url: input.public_url.clone(),
        cors_origins: cors_serialized,
        secure_connections: input.secure_connections.clone(),
        transcoder_reaper_idle_threshold_ms: input.transcoder_reaper_idle_threshold_ms,
        max_remote_streams_per_user: input.max_remote_streams_per_user,
        lan_networks: input.lan_networks.clone(),
        auth_bypass_cidrs: input.auth_bypass_cidrs.clone(),
        ..Default::default()
    };
    let updated = queries::update_server_settings(&state.pool, Some(actor.id), patch)
        .await
        .map_err(ApiError::Internal)?;

    {
        let mut guard = state.settings.write().await;
        *guard = updated.clone();
    }

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "network.update".into(),
            target_kind: Some("settings".into()),
            target_id: Some("1".into()),
            payload_json: serde_json::to_string(&input).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;

    Ok(Json(response_from(updated)))
}

#[derive(Debug, Serialize)]
pub struct ReachabilityResponse {
    pub ok: bool,
    pub public_url: Option<String>,
    pub status_code: Option<u16>,
    pub latency_ms: Option<i64>,
    pub error: Option<String>,
}

pub async fn test_reachability(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<ReachabilityResponse>, ApiError> {
    let s = state.settings.read().await.clone();
    let Some(public_url) = s.public_url.clone() else {
        return Ok(Json(ReachabilityResponse {
            ok: false,
            public_url: None,
            status_code: None,
            latency_ms: None,
            error: Some("public_url is not set".into()),
        }));
    };

    let target = format!("{}/api/v1/health", public_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .danger_accept_invalid_certs(false)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Ok(Json(ReachabilityResponse {
                ok: false,
                public_url: Some(public_url),
                status_code: None,
                latency_ms: None,
                error: Some(format!("client build failed: {e}")),
            }));
        }
    };
    let started = Instant::now();
    match client.get(&target).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let ok = status == 200;
            Ok(Json(ReachabilityResponse {
                ok,
                public_url: Some(public_url),
                status_code: Some(status),
                latency_ms: Some(started.elapsed().as_millis() as i64),
                error: if ok { None } else { Some(format!("HTTP {status}")) },
            }))
        }
        Err(e) => Ok(Json(ReachabilityResponse {
            ok: false,
            public_url: Some(public_url),
            status_code: None,
            latency_ms: Some(started.elapsed().as_millis() as i64),
            error: Some(format!("{e}")),
        })),
    }
}
