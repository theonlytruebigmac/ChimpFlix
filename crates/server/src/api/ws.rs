//! /api/v1/ws — single WebSocket carrying every event.
//!
//! Each connected socket gets the events its subscriber is allowed to
//! see. The `Sessions` event in particular is filtered per-subscriber
//! so non-admin users only see their own active transcodes — without
//! this, every signed-in user could passively observe every other
//! user's playback activity (audit finding, 2026-05-18).
//!
//! Auth is enforced by the upgrade handshake — the browser sends the
//! session cookie on the GET that upgrades. We also reject the upgrade
//! when the request has a foreign `Origin` header (a malicious page
//! that opened a WebSocket via fetch). SameSite cookies don't apply to
//! the WS handshake the way they do to fetch, so we add the explicit
//! Origin gate.

use axum::extract::State;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chimpflix_library::UserRole;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

use crate::auth::AuthUser;
use crate::events::{Event, SessionsEvent};
use crate::state::AppState;

// Per-frame and per-message caps for the live event WebSocket. Inbound
// messages from this socket are limited to ping/pong + the occasional
// future client-to-server control message — none of which need to be
// large. WEEK 1 #12 in `docs/PUBLIC_RELEASE_HARDENING.md`: prevent a
// client (compromised or malicious) from queuing multi-MB frames that
// the server has to buffer.
//
// 64 KiB / 16 KiB matches what tungstenite uses by default but axum
// 0.8 sets higher ceilings (16 MiB / 16 MiB). Pinning them down here
// is cheap.
const WS_MAX_MESSAGE_BYTES: usize = 64 * 1024;
const WS_MAX_FRAME_BYTES: usize = 16 * 1024;

/// Maximum simultaneous WebSocket connections per authenticated user.
/// A real browser opens one to two (main app tab + the rare second
/// tab). 5 is comfortably above legitimate use and well below the
/// point where a misbehaving / malicious client can fan out events
/// at us. MONTH 1 in `docs/PUBLIC_RELEASE_HARDENING.md`.
const WS_MAX_CONNECTIONS_PER_USER: u32 = 5;

pub async fn handler(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !origin_allowed(&state, &headers).await {
        warn!(user_id = user.id, "ws upgrade rejected: foreign Origin");
        return (StatusCode::FORBIDDEN, "origin not permitted").into_response();
    }
    // Per-user connection cap. Claim a slot BEFORE accepting the
    // upgrade so we never half-open a socket that the run() loop
    // would then have to close.
    if !state
        .try_acquire_ws_connection(user.id, WS_MAX_CONNECTIONS_PER_USER)
        .await
    {
        warn!(
            user_id = user.id,
            cap = WS_MAX_CONNECTIONS_PER_USER,
            "ws upgrade rejected: per-user connection cap reached",
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "too many WebSocket connections for this user",
        )
            .into_response();
    }
    ws.max_message_size(WS_MAX_MESSAGE_BYTES)
        .max_frame_size(WS_MAX_FRAME_BYTES)
        .on_upgrade(move |socket| run(state, user, socket))
        .into_response()
}

async fn run(state: AppState, user: AuthUser, mut socket: WebSocket) {
    debug!(user_id = user.id, role = ?user.role, "ws connection opened");
    // RAII guard for the per-user connection cap claimed in `handler`.
    // Releases the slot on every exit path including a panic inside
    // the select loop below. Without it the per-user count leaks and
    // legitimate reconnects would be rejected after the cap is hit.
    struct ConnCountGuard {
        state: AppState,
        user_id: i64,
    }
    impl Drop for ConnCountGuard {
        fn drop(&mut self) {
            let state = self.state.clone();
            let user_id = self.user_id;
            tokio::spawn(async move { state.release_ws_connection(user_id).await });
        }
    }
    let _conn_guard = ConnCountGuard {
        state: state.clone(),
        user_id: user.id,
    };

    let mut rx = state.hub.subscribe();

    // Push the current active-sessions list immediately so freshly-
    // connected dashboards don't have to wait for the next change.
    // Per-user filter: non-admin sees only their own sessions.
    let initial_sessions = filter_sessions(state.transcoder.list_sessions(), &user);
    let initial = SessionsEvent::snapshot(initial_sessions);
    if let Ok(json) = serde_json::to_string(&initial) {
        if socket
            .send(Message::Text(Utf8Bytes::from(json)))
            .await
            .is_err()
        {
            return;
        }
    }

    loop {
        tokio::select! {
            recv = rx.recv() => {
                match recv {
                    Ok(event) => {
                        let json = match serialize_for(&event, &user) {
                            Ok(Some(s)) => s,
                            Ok(None) => continue,
                            Err(e) => {
                                warn!(error = %e, "serialize event failed");
                                continue;
                            }
                        };
                        if socket.send(Message::Text(Utf8Bytes::from(json))).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        warn!(skipped = n, "WS subscriber lagging, dropped events");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Ping(b))) => {
                        let _ = socket.send(Message::Pong(b)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {
                        // Ignore client-to-server messages for now.
                    }
                    Some(Err(e)) => {
                        debug!(error = %e, "ws recv error");
                        break;
                    }
                }
            }
        }
    }
    debug!("ws connection closed");
}

/// Reject WS upgrades whose Origin doesn't match `public_url` /
/// `cors_origins`. Without a check here, a malicious page can open a
/// WebSocket to `/api/v1/ws` via fetch and receive every event the hub
/// broadcasts (cookies are sent on WS handshakes regardless of
/// SameSite=Lax for top-level navigations).
async fn origin_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());
    let Some(origin) = origin else {
        // No Origin header — typically a non-browser client (CLI tooling,
        // server-side scripts). Allow; the auth cookie still gates.
        return true;
    };
    let s = state.settings.read().await;
    if let Some(public_url) = s.public_url.as_deref() {
        if origin_matches(public_url, &origin) {
            return true;
        }
    }
    let allow_list: Vec<String> = serde_json::from_str(&s.cors_origins).unwrap_or_default();
    allow_list
        .iter()
        .any(|entry| entry.trim().eq_ignore_ascii_case(&origin))
}

fn origin_matches(public_url: &str, origin: &str) -> bool {
    // Strip path from public_url so we compare scheme+host+port only.
    let host_only = match public_url.find("://") {
        Some(idx) => {
            let after = &public_url[idx + 3..];
            let host_end = after.find('/').unwrap_or(after.len());
            format!("{}{}", &public_url[..idx + 3], &after[..host_end])
        }
        None => public_url.to_string(),
    };
    host_only.eq_ignore_ascii_case(origin)
}

fn filter_sessions(
    sessions: Vec<chimpflix_transcoder::SessionSnapshot>,
    user: &AuthUser,
) -> Vec<chimpflix_transcoder::SessionSnapshot> {
    if user.role.is_admin_or_owner() {
        return sessions;
    }
    sessions
        .into_iter()
        .filter(|s| s.user_id == user.id)
        .collect()
}

fn serialize_for(event: &Event, user: &AuthUser) -> anyhow::Result<Option<String>> {
    match event {
        Event::Scan(scan) => {
            // Scan progress is admin-only — non-admins shouldn't see
            // operator-side library activity.
            if user.role.is_admin_or_owner() {
                Ok(Some(serde_json::to_string(scan)?))
            } else {
                Ok(None)
            }
        }
        Event::Sessions(s) => {
            let filtered = SessionsEvent {
                kind: s.kind,
                active: s
                    .active
                    .iter()
                    .filter(|snap| user.role.is_admin_or_owner() || snap.user_id == user.id)
                    .cloned()
                    .collect(),
            };
            Ok(Some(serde_json::to_string(&filtered)?))
        }
        Event::Refresh(r) => {
            // `playstate_changed` is private to its user; `library_changed`
            // (user_id = None) goes to everyone. The client re-fetch is
            // access-filtered server-side, so a broadcast leaks nothing.
            match r.user_id {
                Some(uid) if uid != user.id => Ok(None),
                _ => Ok(Some(serde_json::to_string(r)?)),
            }
        }
        // Webhook events are an internal pub/sub variant; not forwarded
        // to WebSocket clients today.
        Event::Webhook(_) => Ok(None),
    }
}

// Reference UserRole so the matcher doesn't drop the unused import on
// non-admin paths — and so this stays grep-able for future audits.
#[allow(dead_code)]
fn _role_static_check(r: UserRole) -> bool {
    r.is_admin_or_owner()
}
