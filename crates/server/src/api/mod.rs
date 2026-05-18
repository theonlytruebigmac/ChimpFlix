//! Router assembly.

mod admin;
mod auth;
mod collections;
mod cors;
mod csrf;
mod episodes;
mod two_factor;
pub mod error;
mod health;
mod items;
mod libraries;
mod markers;
mod my_list;
mod notifications;
mod play_state;
mod prefs;
mod previews;
pub mod rate_limit;
mod scans;
mod seasons;
mod security_headers;
mod stream;
mod subtitles;
mod tags;
mod trakt;
mod ws;

use axum::Router;
use axum::middleware;
use axum::routing::{get, post};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Hard ceiling for any request body. Multipart upload handlers (posters
/// / backdrops, up to 8 MiB) enforce their own stricter caps internally;
/// this is purely a defense-in-depth ceiling so a hostile peer can't
/// stream a multi-gigabyte body and exhaust the receive buffer before
/// the handler even sees it.
const DEFAULT_BODY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

pub fn router(state: AppState) -> Router {
    let auth_lim = rate_limit::auth_limiter();

    // Routes that should be rate-limited per-IP. These are the entry
    // points for credential stuffing, invite scraping, and reset abuse.
    // Built as a separate router so the layer applies only to them.
    let limited_auth = Router::new()
        .route("/auth/login", post(auth::login))
        .route("/auth/register", post(auth::register))
        .route("/auth/setup", post(auth::setup))
        .route("/auth/2fa/login", post(two_factor::challenge_login))
        .route(
            "/auth/password-reset/request",
            post(auth::request_password_reset),
        )
        .route(
            "/auth/password-reset/confirm",
            post(auth::confirm_password_reset),
        )
        .route_layer(middleware::from_fn_with_state(
            auth_lim.clone(),
            rate_limit::enforce,
        ));

    let v1 = Router::new()
        .route("/health", get(health::health))
        .route("/server-info", get(health::server_info))
        // Auth (non-rate-limited surface: status read, logout, identity)
        .route("/auth/status", get(auth::status))
        .route("/auth/logout", post(auth::logout))
        .route(
            "/auth/sessions/revoke-others",
            post(auth::revoke_other_sessions),
        )
        .route("/auth/me/sessions", get(auth::list_my_sessions))
        .route(
            "/auth/me/sessions/{id}",
            axum::routing::delete(auth::revoke_my_session),
        )
        .route("/auth/me", get(auth::me).patch(auth::update_me))
        .route("/auth/me/password", post(auth::change_password))
        .route(
            "/auth/me/email/request-change",
            post(auth::request_email_change),
        )
        .route(
            "/auth/me/email/confirm",
            post(auth::confirm_email_change),
        )
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
            "/auth/invites/{id}",
            axum::routing::delete(auth::revoke_invite),
        )
        // 2FA management (authenticated user — own account)
        .route("/auth/2fa/status", get(two_factor::status))
        .route("/auth/2fa/enroll", post(two_factor::enroll))
        .route("/auth/2fa/verify", post(two_factor::verify))
        .route("/auth/2fa/disable", post(two_factor::disable))
        .route(
            "/auth/2fa/recovery-codes/regenerate",
            post(two_factor::regenerate_recovery_codes),
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
        .route("/libraries/{id}/stats", get(libraries::library_stats))
        .route("/libraries/{id}/verify", post(libraries::verify_library))
        .route("/libraries/{id}/purge", post(libraries::purge_library))
        .route(
            "/libraries/{id}/access",
            get(libraries::get_access).put(libraries::put_access),
        )
        .route(
            "/libraries/{id}/detect-markers",
            post(markers::detect_for_library),
        )
        .route(
            "/libraries/{id}/refresh-metadata",
            post(libraries::refresh_metadata),
        )
        .route(
            "/libraries/{id}/generate-previews",
            post(libraries::generate_previews),
        )
        .route(
            "/items/{id}/detect-markers",
            post(markers::detect_for_item),
        )
        // Scan jobs
        .route("/scans/{id}", get(scans::get_one))
        // Items / seasons / episodes
        .route("/items", get(items::list))
        .route("/items/trending", get(items::trending))
        .route("/items/{id}", get(items::get_one).patch(items::patch_item))
        .route("/items/{id}/trailer", get(items::trailer))
        .route("/items/{id}/similar", get(items::similar))
        .route("/items/{id}/refresh", post(items::refresh))
        .route("/items/{id}/match-search", get(items::match_search))
        .route("/items/{id}/match-apply", post(items::match_apply))
        .route("/items/{id}/reviews", get(items::list_reviews))
        .route("/tags", get(tags::list_all))
        .route(
            "/items/{id}/tags",
            get(tags::list_for_item).post(tags::add_to_item),
        )
        .route(
            "/items/{id}/tags/{tag_id}",
            axum::routing::delete(tags::remove_from_item),
        )
        // Trakt link/sync
        .route("/trakt/link/start", post(trakt::link_start))
        .route("/trakt/link/poll", post(trakt::link_poll))
        .route("/trakt/status", get(trakt::status))
        .route("/trakt/unlink", post(trakt::unlink))
        .route("/trakt/sync-now", post(trakt::sync_now))
        // Per-user ratings (Trakt-synced when linked)
        .route(
            "/items/{id}/rating",
            get(trakt::get_item_rating)
                .put(trakt::put_item_rating)
                .delete(trakt::delete_item_rating),
        )
        .route(
            "/episodes/{id}/rating",
            get(trakt::get_episode_rating)
                .put(trakt::put_episode_rating)
                .delete(trakt::delete_episode_rating),
        )
        .route(
            "/items/{id}/external-subtitles",
            get(subtitles::list_for_item),
        )
        .route(
            "/episodes/{id}/external-subtitles",
            get(subtitles::list_for_episode),
        )
        .route("/external-subtitles/{id}/file", get(subtitles::serve_file))
        .route(
            "/media-files/{id}/preview/manifest",
            get(previews::manifest),
        )
        .route("/media-files/{id}/preview/sprite", get(previews::sprite))
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
        .route("/stream/prewarm", post(stream::prewarm_session))
        .route(
            "/stream/sessions/{id}",
            axum::routing::delete(stream::delete_session),
        )
        .route("/stream/sessions/{id}/pause", post(stream::pause_session))
        .route("/stream/sessions/{id}/resume", post(stream::resume_session))
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
        // Notifications (per-user inbox)
        .route("/notifications", get(notifications::list))
        .route("/notifications/unread-count", get(notifications::unread_count))
        .route(
            "/notifications/{id}/read",
            post(notifications::mark_read),
        )
        .route(
            "/notifications/read-all",
            post(notifications::mark_all_read),
        )
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
        .route(
            "/admin/settings/email",
            get(admin::email::get_status),
        )
        .route(
            "/admin/settings/email/password",
            axum::routing::put(admin::email::set_password)
                .delete(admin::email::clear_password),
        )
        .route(
            "/admin/settings/email/test",
            post(admin::email::test),
        )
        .route("/admin/audit", get(admin::audit::list))
        .route("/admin/library-health", get(admin::health::get))
        .route("/admin/agents", get(admin::agents::list_available))
        .route("/admin/secrets", get(admin::secrets::list))
        .route(
            "/admin/secrets/{name}",
            axum::routing::put(admin::secrets::put).delete(admin::secrets::delete),
        )
        .route("/admin/secrets/{name}/test", post(admin::secrets::test))
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
            "/admin/users/{id}/2fa/reset",
            post(admin::users::reset_user_totp),
        )
        .route(
            "/admin/users/{id}/unlock-attempts",
            post(admin::users::unlock_login_attempts),
        )
        .route(
            "/admin/access",
            get(admin::users::get_access_matrix).put(admin::users::put_access_matrix),
        )
        .route(
            "/admin/access-groups",
            get(admin::access_groups::list).post(admin::access_groups::create),
        )
        .route(
            "/admin/access-groups/{id}",
            get(admin::access_groups::get_one)
                .patch(admin::access_groups::update)
                .delete(admin::access_groups::delete),
        )
        .route(
            "/admin/access-groups/{id}/libraries",
            axum::routing::put(admin::access_groups::set_libraries),
        )
        .route(
            "/admin/access-groups/{id}/members",
            axum::routing::put(admin::access_groups::set_members),
        )
        .route(
            "/admin/users/{id}/access-groups",
            get(admin::access_groups::get_user_groups)
                .put(admin::access_groups::set_user_groups),
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
        // One-click instance-wide maintenance
        .route(
            "/admin/maintenance/verify-all",
            post(admin::maintenance::verify_all),
        )
        .route(
            "/admin/maintenance/purge-all",
            post(admin::maintenance::purge_all),
        )
        .route(
            "/admin/maintenance/vacuum",
            post(admin::maintenance::vacuum_database),
        )
        .route(
            "/admin/maintenance/clear-transcode-cache",
            post(admin::maintenance::clear_transcode_cache),
        )
        // WebSocket
        .route("/ws", get(ws::handler));

    Router::new()
        .route("/health", get(health::health))
        .nest("/api/v1", v1.merge(limited_auth))
        // Cap JSON body size for safety; multipart routes set their own
        // per-handler limits via Multipart's `max_length`.
        .layer(RequestBodyLimitLayer::new(DEFAULT_BODY_LIMIT_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            csrf::layer,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers::layer,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            cors::layer,
        ))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
