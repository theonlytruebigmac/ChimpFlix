//! Router assembly.

mod auth;
mod episodes;
pub mod error;
mod health;
mod items;
mod libraries;
mod play_state;
mod scans;
mod seasons;
mod stream;
mod ws;

use axum::Router;
use axum::routing::{get, post};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let v1 = Router::new()
        .route("/health", get(health::health))
        .route("/server-info", get(health::server_info))
        // Auth
        .route("/auth/status", get(auth::status))
        .route("/auth/setup", post(auth::setup))
        .route("/auth/login", post(auth::login))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::me))
        .route("/auth/register", post(auth::register))
        .route(
            "/auth/invites",
            get(auth::list_invites).post(auth::create_invite),
        )
        .route(
            "/auth/invites/{code}",
            axum::routing::delete(auth::revoke_invite),
        )
        // Libraries
        .route("/libraries", get(libraries::list).post(libraries::create))
        .route(
            "/libraries/{id}",
            get(libraries::get_one)
                .patch(libraries::update)
                .delete(libraries::delete_one),
        )
        .route("/libraries/{id}/scan", post(libraries::trigger_scan))
        .route("/libraries/{id}/scans", get(libraries::list_scans))
        // Scan jobs
        .route("/scans/{id}", get(scans::get_one))
        // Items / seasons / episodes
        .route("/items", get(items::list))
        .route("/items/{id}", get(items::get_one))
        .route("/seasons/{id}", get(seasons::get_one))
        .route("/episodes/{id}", get(episodes::get_one))
        // Streaming
        .route("/stream/{file_id}/direct", get(stream::direct))
        .route("/stream/sessions", post(stream::create_session))
        .route(
            "/stream/sessions/{id}",
            axum::routing::delete(stream::delete_session),
        )
        .route(
            "/stream/sessions/{id}/master.m3u8",
            get(stream::master_playlist),
        )
        .route(
            "/stream/sessions/{id}/{variant}/{name}",
            get(stream::variant_file),
        )
        // Play state
        .route("/play-state", post(play_state::update))
        .route("/play-state/scrobble", post(play_state::scrobble))
        .route("/play-state/on-deck", get(play_state::on_deck))
        // WebSocket
        .route("/ws", get(ws::handler));

    Router::new()
        .route("/health", get(health::health))
        .nest("/api/v1", v1)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
