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
    /// Operator-pinned listen socket (overrides the BIND_ADDR env at
    /// runtime). Must be in the response — the admin form binds a
    /// text input to this value and `bind_interface.trim()` on save
    /// would explode with `cannot access property 'trim' of
    /// undefined` if the field was missing from the JSON. (The bug
    /// presented as "Save failed: can't access property trim, k is
    /// undefined" when adjusting the CORS allowlist, because the
    /// page-load fetch didn't include this key.)
    pub bind_interface: String,
    /// Diagnostic block (WEEK 1 #8 in
    /// `docs/PUBLIC_RELEASE_HARDENING.md`). Surfaces what the server
    /// actually trusts and what it sees as the request peer, so the
    /// operator notices a misconfigured proxy before it silently
    /// collapses per-IP rate limits.
    pub proxy_diagnostic: ProxyDiagnostic,
}

#[derive(Debug, Serialize)]
pub struct ProxyDiagnostic {
    /// `TRUSTED_PROXIES` parsed at boot, formatted as CIDR strings.
    /// Empty array = no proxy headers are honoured at all.
    pub trusted_proxies: Vec<String>,
    /// Immediate TCP peer IP for the request that loaded this page.
    /// `None` when the extractor couldn't resolve a peer (shouldn't
    /// happen on a normal axum boot).
    pub peer_ip: Option<String>,
    /// True when the peer IP belongs to an RFC1918 / RFC4193 /
    /// loopback range — almost certainly a Docker bridge or a
    /// reverse proxy. Combined with the trusted-proxies list, the
    /// UI uses this to render an actionable warning:
    /// "your proxy sits at 172.18.0.x, but TRUSTED_PROXIES doesn't
    /// include that — every request looks like it's coming from
    /// the same IP, so per-IP rate limits collapse to one bucket."
    pub peer_is_private: bool,
    /// True when `peer_is_private` is true AND `trusted_proxies` is
    /// either empty or does not contain `peer_ip`. The UI banner
    /// fires off this flag.
    pub looks_misconfigured: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_interface: Option<String>,
}

fn response_from(
    s: chimpflix_library::ServerSettings,
    proxy_diagnostic: ProxyDiagnostic,
) -> NetworkResponse {
    let cors_origins: Vec<String> = serde_json::from_str(&s.cors_origins).unwrap_or_default();
    NetworkResponse {
        public_url: s.public_url,
        cors_origins,
        secure_connections: s.secure_connections,
        transcoder_reaper_idle_threshold_ms: s.transcoder_reaper_idle_threshold_ms,
        max_remote_streams_per_user: s.max_remote_streams_per_user,
        lan_networks: s.lan_networks,
        auth_bypass_cidrs: s.auth_bypass_cidrs,
        bind_interface: s.bind_interface,
        proxy_diagnostic,
    }
}

/// True for RFC1918 / RFC4193 / loopback / link-local. These are the
/// ranges a reverse proxy or Docker bridge will sit in; if the
/// immediate peer is in one of them, the operator almost certainly
/// has a proxy in front and the rate-limiter / audit log will give
/// useless attribution unless `TRUSTED_PROXIES` covers it.
fn is_private_peer(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

fn build_proxy_diagnostic(state: &AppState, peer: std::net::IpAddr) -> ProxyDiagnostic {
    let trusted: Vec<String> = state
        .trusted_proxies
        .iter()
        .map(|net| net.to_string())
        .collect();
    let peer_is_private = is_private_peer(peer);
    let peer_in_trusted = state.trusted_proxies.iter().any(|net| net.contains(&peer));
    ProxyDiagnostic {
        trusted_proxies: trusted,
        peer_ip: Some(peer.to_string()),
        peer_is_private,
        looks_misconfigured: peer_is_private && !peer_in_trusted,
    }
}

pub async fn get(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> Result<Json<NetworkResponse>, ApiError> {
    let s = state.settings.read().await.clone();
    let diag = build_proxy_diagnostic(&state, peer.ip());
    Ok(Json(response_from(s, diag)))
}

pub async fn patch(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<NetworkUpdate>,
) -> Result<Json<NetworkResponse>, ApiError> {
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
        bind_interface: input.bind_interface.clone(),
        ..Default::default()
    };
    // All validation is centralized in
    // `crate::api::admin::settings::validate` so the catch-all and
    // network-fragment PATCH endpoints agree, and a new endpoint
    // picks up future field checks for free.
    crate::api::admin::settings::validate(&patch)?;
    let updated = queries::update_server_settings(&state.pool, Some(actor.id), patch)
        .await
        .map_err(ApiError::Internal)?;

    {
        let mut guard = state.settings.write().await;
        *guard = updated.clone();
    }
    let diag = build_proxy_diagnostic(&state, peer.ip());

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

    Ok(Json(response_from(updated, diag)))
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
    // SSRF guard. Without it, an owner can set `public_url` to an
    // internal URL and use the reachability test as a port-scan oracle
    // (200 vs connection-refused vs timeout reveals which internal
    // ports are open).
    if let Err(reason) = crate::ssrf::ensure_safe_outbound_url(&target).await {
        return Ok(Json(ReachabilityResponse {
            ok: false,
            public_url: Some(public_url),
            status_code: None,
            latency_ms: None,
            error: Some(format!("blocked: {reason}")),
        }));
    }
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
                error: if ok {
                    None
                } else {
                    Some(format!("HTTP {status}"))
                },
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
