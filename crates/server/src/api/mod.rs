//! Router assembly.

pub(crate) mod access;
pub(crate) mod admin;
mod auth;
mod collections;
mod cors;
mod csrf;
mod episodes;
pub mod error;
mod health;
mod items;
mod libraries;
pub(crate) mod markers;
mod my_list;
mod notifications;
mod play_state;
mod prefs;
pub mod rate_limit;
mod scans;
mod seasons;
mod security_headers;
mod stream;
mod subtitles;
mod tags;
mod trakt;
mod two_factor;
mod ws;

use std::time::Duration;

use axum::Router;
use axum::middleware;
use axum::routing::{get, post};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

/// Hard ceiling for any request body. Multipart upload handlers (posters
/// / backdrops, up to 8 MiB) enforce their own stricter caps internally;
/// this is purely a defense-in-depth ceiling so a hostile peer can't
/// stream a multi-gigabyte body and exhaust the receive buffer before
/// the handler even sees it.
const DEFAULT_BODY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// Per-route body cap for the auth subrouter — login, register, reset
/// confirm, 2FA challenge. Every legitimate payload here is a few
/// hundred bytes; 16 KiB is plenty for inflated JSON and future
/// fields, and stops a hostile peer from POSTing 16 MiB to the login
/// endpoint just to make the server allocate.
const AUTH_BODY_LIMIT_BYTES: usize = 16 * 1024;

/// Hard ceiling for request lifetime on the non-streaming admin/API
/// surface. Without this, a slowloris-style client can pin a worker
/// indefinitely by trickling bytes. The streaming routes (HLS
/// segments, WebSocket) are mounted outside this layer because they
/// genuinely run for the duration of a playback / live connection.
const REQUEST_TIMEOUT_SECS: u64 = 60;

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
        // Tight per-route body cap on the auth surface. Every payload
        // here is a few hundred bytes at most (username + password +
        // a 64-char token); 16 KiB leaves room for inflated JSON and
        // future fields without ever inviting a multi-MB attack body
        // against the routes that gate access to the system. The
        // global 16 MiB cap further out is the defense for everything
        // else that isn't an upload route.
        .layer(RequestBodyLimitLayer::new(AUTH_BODY_LIMIT_BYTES))
        // Tight per-request timeout on the auth surface. Login /
        // register / reset bodies are tiny (KB at most); 60s is
        // generous. The cap prevents slowloris-style attacks against
        // exactly the routes that gate access to the system. 503 is
        // the right status for "the server gave up waiting on you".
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Duration::from_secs(REQUEST_TIMEOUT_SECS),
        ))
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
        .route("/auth/me/email/confirm", post(auth::confirm_email_change))
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
        .route("/items/{id}/detect-markers", post(markers::detect_for_item))
        // Per-media-file marker editor (operator-only) — used by the
        // owner-side marker-correction UI. The player-side surface
        // gets markers via the item detail response and never edits.
        .route(
            "/media-files/{id}/markers",
            get(markers::list_for_media_file).put(markers::replace_manual),
        )
        // Scan jobs
        .route("/scans/{id}", get(scans::get_one))
        // Items / seasons / episodes
        .route("/items", get(items::list))
        .route("/items/trending", get(items::trending))
        .route("/items/{id}", get(items::get_one).patch(items::patch_item))
        .route(
            "/items/{id}/media",
            axum::routing::delete(items::delete_item_media),
        )
        .route("/items/{id}/trailer", get(items::trailer))
        .route("/items/{id}/similar", get(items::similar))
        .route("/items/{id}/refresh", post(items::refresh))
        .route("/items/{id}/match-search", get(items::match_search))
        .route("/items/{id}/match-apply", post(items::match_apply))
        .route("/items/{id}/match-clear", post(items::match_clear))
        .route("/items/{id}/merge-into", post(items::merge_into))
        .route("/items/{id}/report-issue", post(items::report_issue))
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
        .route(
            "/episodes/{id}/media",
            axum::routing::delete(items::delete_episode_media),
        )
        // Streaming
        .route("/stream/{file_id}/direct", get(stream::direct))
        .route("/stream/sessions", post(stream::create_session))
        .route("/stream/prewarm", post(stream::prewarm_session))
        .route(
            "/stream/sessions/{id}",
            axum::routing::delete(stream::delete_session),
        )
        // POST alias for the DELETE — exists because navigator.sendBeacon()
        // only supports POST, and sendBeacon is the most reliable way to
        // fire a teardown request as the page is being unloaded (PWA
        // force-close in particular, where fetch+keepalive can be dropped).
        .route("/stream/sessions/{id}/close", post(stream::delete_session))
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
        .route("/play-state/config", get(play_state::config))
        .route("/play-state/scrobble", post(play_state::scrobble))
        .route("/play-state/event", post(play_state::event))
        .route("/play-state/watched", post(play_state::set_watched))
        .route("/play-state/on-deck", get(play_state::on_deck))
        .route("/play-state/history", get(play_state::history))
        // Collections (movie franchises + admin-curated manual collections)
        .route("/collections", get(collections::list))
        .route("/collections/{id}", get(collections::get_one))
        .route(
            "/collections/{id}/poster/blob",
            get(admin::collections::get_poster_blob),
        )
        .route(
            "/collections/{id}/backdrop/blob",
            get(admin::collections::get_backdrop_blob),
        )
        // Admin CRUD for manual collections (auto collections are read-only)
        .route("/admin/collections", post(admin::collections::create))
        .route(
            "/admin/collections/{id}",
            axum::routing::patch(admin::collections::update).delete(admin::collections::delete),
        )
        .route(
            "/admin/collections/{id}/items",
            post(admin::collections::add_items).put(admin::collections::reorder),
        )
        .route(
            "/admin/collections/{id}/items/{item_id}",
            axum::routing::delete(admin::collections::remove_item),
        )
        .route(
            "/admin/collections/{id}/poster",
            post(admin::collections::upload_poster),
        )
        .route(
            "/admin/collections/{id}/backdrop",
            post(admin::collections::upload_backdrop),
        )
        .route(
            "/admin/smart-collections",
            post(admin::collections::create_smart),
        )
        .route(
            "/admin/smart-collections/{id}/rule",
            axum::routing::put(admin::collections::update_smart_rule),
        )
        // Pre-roll video (operator-uploaded; plays before each session)
        .route(
            "/admin/preroll",
            get(admin::preroll::get_status)
                .post(admin::preroll::upload)
                .delete(admin::preroll::clear),
        )
        .route("/preroll/blob", get(admin::preroll::serve_blob))
        // Bulk item operations
        .route(
            "/admin/items/bulk/refresh-metadata",
            post(admin::bulk::refresh_metadata),
        )
        .route("/admin/items/bulk/add-tag", post(admin::bulk::add_tag))
        .route(
            "/admin/items/bulk/remove-tag",
            post(admin::bulk::remove_tag),
        )
        .route(
            "/admin/items/bulk/detect-markers",
            post(admin::bulk::detect_markers),
        )
        // Background job queue (Owner-only) — durable pipeline jobs
        // for marker detection, loudness analysis, subtitle fetch,
        // ratings, season fingerprint bootstrap. The list endpoint
        // also accepts ?kind=... and ?status=... query params for
        // filtering.
        .route("/admin/jobs", get(admin::jobs::list))
        .route("/admin/jobs/summary", get(admin::jobs::summary))
        .route("/admin/jobs/{id}/requeue", post(admin::jobs::requeue))
        .route(
            "/admin/jobs/process-all-pending",
            post(admin::jobs::process_all_pending),
        )
        .route(
            "/admin/jobs/queued",
            axum::routing::delete(admin::jobs::wipe_queued),
        )
        .route(
            "/admin/jobs/dead",
            axum::routing::delete(admin::jobs::clear_dead),
        )
        // Notifications (per-user inbox)
        .route("/notifications", get(notifications::list))
        .route(
            "/notifications/unread-count",
            get(notifications::unread_count),
        )
        .route("/notifications/{id}/read", post(notifications::mark_read))
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
            get(prefs::get_hidden_libraries).put(prefs::put_hidden_libraries),
        )
        // Admin (owner-only)
        .route("/admin/backup", post(admin::backup::backup))
        .route("/admin/backups", get(admin::backup::list))
        .route(
            "/admin/backups/cancel-restore",
            post(admin::backup::cancel_restore),
        )
        .route(
            "/admin/backups/{filename}/download",
            get(admin::backup::download),
        )
        .route(
            "/admin/backups/{filename}",
            axum::routing::delete(admin::backup::delete),
        )
        .route(
            "/admin/backups/{filename}/stage-restore",
            post(admin::backup::stage_restore),
        )
        .route("/admin/dashboard", get(admin::dashboard::get))
        .route("/admin/stats/overview", get(admin::stats::overview))
        .route("/admin/stats/activity", get(admin::stats::activity))
        .route("/admin/stats/top-users", get(admin::stats::top_users))
        .route("/admin/stats/top-items", get(admin::stats::top_items))
        .route(
            "/admin/stats/top-platforms",
            get(admin::stats::top_platforms),
        )
        .route(
            "/admin/stats/top-libraries",
            get(admin::stats::top_libraries),
        )
        .route("/admin/stats/now-playing", get(admin::stats::now_playing))
        .route(
            "/admin/stats/plays-per-day",
            get(admin::stats::plays_per_day),
        )
        .route(
            "/admin/stats/plays-per-hour",
            get(admin::stats::plays_per_hour),
        )
        .route(
            "/admin/settings",
            get(admin::settings::get).patch(admin::settings::patch),
        )
        .route("/admin/settings/email", get(admin::email::get_status))
        .route(
            "/admin/settings/email/password",
            axum::routing::put(admin::email::set_password).delete(admin::email::clear_password),
        )
        .route("/admin/settings/email/test", post(admin::email::test))
        .route("/admin/audit", get(admin::audit::list))
        .route("/admin/library-health", get(admin::health::get))
        .route("/admin/library-health/items", get(admin::health::items))
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
        // Registry-driven overview (new admin UI in Phase 7). Sibling
        // to the legacy `/admin/tasks` CRUD surface; the existing
        // advanced-editor view still uses the row-based endpoints.
        .route(
            "/admin/tasks/overview",
            get(admin::tasks_overview::overview),
        )
        .route("/admin/tasks/summary", get(admin::tasks_overview::summary))
        .route(
            "/admin/tasks/activity",
            get(admin::tasks_overview::activity),
        )
        .route(
            "/admin/tasks/kind/{kind}",
            get(admin::tasks_overview::kind_detail)
                .patch(admin::tasks_overview::update_kind_schedule),
        )
        .route(
            "/admin/tasks/kind/{kind}/gate",
            axum::routing::patch(admin::tasks_overview::update_gate),
        )
        .route(
            "/admin/tasks/kind/{kind}/run",
            post(admin::tasks_overview::run_kind_now),
        )
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
        .route(
            "/admin/webhooks/{id}/test",
            post(admin::webhooks::test_fire),
        )
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
            get(admin::users::list_user_sessions).delete(admin::users::revoke_user_sessions),
        )
        .route(
            "/admin/users/{id}/2fa/reset",
            post(admin::users::reset_user_totp),
        )
        .route(
            "/admin/users/{id}/password-reset",
            post(admin::users::send_password_reset),
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
            get(admin::access_groups::get_user_groups).put(admin::access_groups::set_user_groups),
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
        .layer(middleware::from_fn_with_state(state.clone(), csrf::layer))
        // Client-IP resolution runs before csrf + rate-limit + auth so
        // those layers all read from the same authoritative
        // `EffectiveClientIp` extension. Outer than csrf in the chain
        // means it executes earlier in the request.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::client_ip::middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers::layer,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), cors::layer))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
