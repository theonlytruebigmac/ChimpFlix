//! Router assembly.

mod admin;
mod auth;
mod collections;
mod cors;
mod episodes;
pub mod error;
mod health;
mod items;
mod libraries;
mod markers;
mod my_list;
mod play_state;
mod prefs;
mod scans;
mod seasons;
mod stream;
mod ws;

use axum::Router;
use axum::middleware;
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
        .route("/auth/me", get(auth::me).patch(auth::update_me))
        .route("/auth/register", post(auth::register))
        .route("/auth/users", get(auth::list_users))
        .route(
            "/auth/users/{id}",
            axum::routing::patch(auth::update_user).delete(auth::delete_user),
        )
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
        .route(
            "/libraries/{id}/access",
            get(libraries::get_access).put(libraries::put_access),
        )
        .route(
            "/libraries/{id}/detect-markers",
            post(markers::detect_for_library),
        )
        .route(
            "/items/{id}/detect-markers",
            post(markers::detect_for_item),
        )
        // Scan jobs
        .route("/scans/{id}", get(scans::get_one))
        // Items / seasons / episodes
        .route("/items", get(items::list))
        .route("/items/{id}", get(items::get_one).patch(items::patch_item))
        .route("/items/{id}/trailer", get(items::trailer))
        .route("/items/{id}/similar", get(items::similar))
        .route("/items/{id}/refresh", post(items::refresh))
        .route("/items/{id}/match-search", get(items::match_search))
        .route("/items/{id}/match-apply", post(items::match_apply))
        .route("/items/{id}/reviews", get(items::list_reviews))
        .route(
            "/items/{id}/credits",
            axum::routing::patch(items::patch_credits),
        )
        .route("/items/{id}/tmdb-posters", get(items::tmdb_posters))
        .route(
            "/items/{id}/poster/from-tmdb",
            post(items::apply_tmdb_poster),
        )
        .route("/items/{id}/poster", post(items::upload_poster))
        .route("/items/{id}/poster/blob", get(items::get_poster_blob))
        .route("/items/{id}/backdrop", post(items::upload_backdrop))
        .route("/items/{id}/backdrop/blob", get(items::get_backdrop_blob))
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
        .route("/play-state/watched", post(play_state::set_watched))
        .route("/play-state/on-deck", get(play_state::on_deck))
        .route("/play-state/history", get(play_state::history))
        // Collections (movie franchises)
        .route("/collections", get(collections::list))
        .route("/collections/{id}", get(collections::get_one))
        // My List
        .route("/my-list", get(my_list::list))
        .route(
            "/my-list/{item_id}",
            post(my_list::add).delete(my_list::remove),
        )
        // Prefs
        .route(
            "/prefs/hidden-libraries",
            get(prefs::get_hidden_libraries)
                .put(prefs::put_hidden_libraries),
        )
        // Admin (owner-only)
        .route("/admin/backup", post(admin::backup::backup))
        .route("/admin/dashboard", get(admin::dashboard::get))
        .route(
            "/admin/settings",
            get(admin::settings::get).patch(admin::settings::patch),
        )
        .route("/admin/audit", get(admin::audit::list))
        .route("/admin/agents", get(admin::agents::list_available))
        .route(
            "/admin/libraries/{id}/agents",
            get(admin::agents::get_for_library).put(admin::agents::set_for_library),
        )
        .route(
            "/admin/tasks",
            get(admin::tasks::list).post(admin::tasks::create),
        )
        .route(
            "/admin/tasks/{id}",
            axum::routing::patch(admin::tasks::update).delete(admin::tasks::delete),
        )
        .route("/admin/tasks/{id}/run", post(admin::tasks::run_now))
        .route("/admin/tasks/{id}/runs", get(admin::tasks::list_runs))
        .route(
            "/admin/transcoder/capabilities",
            get(admin::transcoder::capabilities),
        )
        .route(
            "/admin/transcoder/presets",
            get(admin::transcoder::list_presets).post(admin::transcoder::create_preset),
        )
        .route(
            "/admin/transcoder/presets/{id}",
            axum::routing::patch(admin::transcoder::update_preset)
                .delete(admin::transcoder::delete_preset),
        )
        .route(
            "/admin/network",
            get(admin::network::get).patch(admin::network::patch),
        )
        .route(
            "/admin/network/test-reachability",
            post(admin::network::test_reachability),
        )
        .route(
            "/admin/webhooks",
            get(admin::webhooks::list).post(admin::webhooks::create),
        )
        .route(
            "/admin/webhooks/{id}",
            axum::routing::patch(admin::webhooks::update).delete(admin::webhooks::delete),
        )
        .route("/admin/webhooks/{id}/test", post(admin::webhooks::test_fire))
        .route(
            "/admin/webhooks/{id}/deliveries",
            get(admin::webhooks::list_deliveries),
        )
        .route("/admin/sessions", get(admin::users::list_sessions))
        .route(
            "/admin/sessions/{id}",
            axum::routing::delete(admin::users::revoke_session),
        )
        .route(
            "/admin/users/{id}/sessions",
            get(admin::users::list_user_sessions)
                .delete(admin::users::revoke_user_sessions),
        )
        .route(
            "/admin/access",
            get(admin::users::get_access_matrix).put(admin::users::put_access_matrix),
        )
        .route(
            "/admin/optimized",
            get(admin::optimized::list).post(admin::optimized::create),
        )
        .route(
            "/admin/optimized/{id}",
            axum::routing::delete(admin::optimized::delete),
        )
        .route("/admin/logs", get(admin::maintenance::logs))
        .route("/admin/alerts", get(admin::maintenance::alerts))
        .route(
            "/admin/privacy",
            get(admin::maintenance::get_privacy).patch(admin::maintenance::patch_privacy),
        )
        // WebSocket
        .route("/ws", get(ws::handler));

    Router::new()
        .route("/health", get(health::health))
        .nest("/api/v1", v1)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            cors::layer,
        ))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
