//! /api/v1/ws — single WebSocket carrying every event.
//!
//! v0.1: no topic subscriptions yet. Every connected socket gets every
//! event the hub broadcasts. Auth is enforced by the upgrade handshake —
//! the browser sends the session cookie on the GET that upgrades.

use axum::extract::State;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

use crate::auth::AuthUser;
use crate::events::{Event, SessionsEvent};
use crate::state::AppState;

pub async fn handler(
    State(state): State<AppState>,
    _user: AuthUser,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run(state, socket))
}

async fn run(state: AppState, mut socket: WebSocket) {
    debug!("ws connection opened");
    let mut rx = state.hub.subscribe();

    // Push the current active-sessions list immediately so freshly-
    // connected dashboards don't have to wait for the next change.
    let initial = SessionsEvent::snapshot(state.transcoder.list_sessions());
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
                        let json = match serialize(&event) {
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

fn serialize(event: &Event) -> anyhow::Result<Option<String>> {
    match event {
        Event::Scan(scan) => Ok(Some(serde_json::to_string(scan)?)),
        Event::Sessions(s) => Ok(Some(serde_json::to_string(s)?)),
        // Webhook events are an internal pub/sub variant; not forwarded
        // to WebSocket clients today.
        Event::Webhook(_) => Ok(None),
    }
}
