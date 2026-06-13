//! `GET /admin/settings` and `PATCH /admin/settings`.
//!
//! The settings cache on `AppState` is the source of truth for *readers*
//! (transcoder, CORS layer, etc.). DB persistence is the source of truth
//! across restarts. On PATCH we (1) write the diff to SQLite, (2) refresh
//! the cache from the canonical row, (3) audit the change.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use chimpflix_library::{NewAuditEntry, ServerSettings, ServerSettingsUpdate, queries};
use serde::Serialize;
use serde_json::json;

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    pub settings: ServerSettings,
    /// Read-only server version (from the server crate's Cargo
    /// package version). For display only — PATCH ignores it.
    pub version: &'static str,
    /// Read-only on-disk DATA_DIR path. For display only — PATCH
    /// ignores it.
    pub data_dir: String,
}

pub async fn get(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<SettingsResponse>, ApiError> {
    // Serve from the cache — it's already populated at startup and kept
    // fresh by `patch`. Cheap clone, no DB round-trip.
    let settings = state.settings.read().await.clone();
    Ok(Json(SettingsResponse {
        settings,
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.data_dir.display().to_string(),
    }))
}

pub async fn patch(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<ServerSettingsUpdate>,
) -> Result<Json<SettingsResponse>, ApiError> {
    validate(&input)?;

    let updated = queries::update_server_settings(&state.pool, Some(actor.id), input.clone())
        .await
        .map_err(ApiError::Internal)?;

    {
        let mut guard = state.settings.write().await;
        *guard = updated.clone();
    }

    // Hot-apply settings whose runtime effect isn't just "next time
    // someone reads the cache." `transcoder_max_background_concurrent`
    // is already hot because the scheduler reads it fresh every tick.
    if let Some(target) = input.job_workers {
        if let Some(handle) = state.worker_pool.read().await.clone() {
            handle.resize(target.max(1) as usize);
        }
    }
    if let Some(raw) = input.job_kind_concurrency.as_deref() {
        // Already validated as well-formed JSON of the right shape
        // above. Re-parse defensively: a future caller could route
        // through this fn without `validate(&input)?` at the top.
        match serde_json::from_str::<std::collections::HashMap<String, usize>>(raw) {
            Ok(overrides) => {
                if let Some(handle) = state.worker_pool.read().await.clone() {
                    handle.apply_kind_concurrency(&overrides).await;
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "job_kind_concurrency parsed at validate-time but failed at hot-apply; skipping live resize",
                );
            }
        }
    }

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let payload = serde_json::to_string(&input).ok();
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "settings.update".into(),
            target_kind: Some("settings".into()),
            target_id: Some("1".into()),
            payload_json: payload,
            ip: None,
            user_agent,
        },
    )
    .await;

    Ok(Json(SettingsResponse {
        settings: updated,
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.data_dir.display().to_string(),
    }))
}

/// Validate every field on a `ServerSettingsUpdate` that has a
/// validation rule attached. Exported so any PATCH endpoint touching
/// `server_settings` (currently the catch-all settings handler, plus
/// the network-specific one, and any future fragment handlers) can
/// route through the same checks instead of duplicating inline.
/// Adding a new field with constraints? Add the check here and every
/// caller picks it up automatically.
pub fn validate(patch: &ServerSettingsUpdate) -> Result<(), ApiError> {
    // SECURITY: `preroll_path` is read by `/preroll/blob` to serve raw
    // bytes from `data/preroll/<path>`. Without sanitisation an admin
    // (or a session-hijacked admin) could PATCH `preroll_path` to
    // `../../etc/passwd` and the serve endpoint would happily stream
    // it back. Restrict to a single filename component (no slashes,
    // no `..`, no leading `.`, no NUL). The upload handler enforces
    // its own narrower rule (preroll.{mp4|webm|mkv}); this is the
    // generic-settings-PATCH belt-and-suspenders.
    if let Some(Some(ref p)) = patch.preroll_path {
        let bad = p.is_empty()
            || p.contains('/')
            || p.contains('\\')
            || p.contains('\0')
            || p.contains("..")
            || p.starts_with('.');
        if bad {
            return Err(ApiError::validation(
                "preroll_path must be a single filename without `..`, slashes, \
                 a leading dot, or NUL bytes",
            ));
        }
    }
    if let Some(ref s) = patch.secure_connections {
        if !matches!(s.as_str(), "required" | "preferred" | "disabled") {
            return Err(ApiError::validation(
                "secure_connections must be one of: required, preferred, disabled",
            ));
        }
    }
    if let Some(ref s) = patch.periodic_scan_frequency {
        if !matches!(
            s.as_str(),
            "every_15_minutes"
                | "every_30_minutes"
                | "hourly"
                | "every_2_hours"
                | "every_6_hours"
                | "every_12_hours"
                | "daily"
        ) {
            return Err(ApiError::validation(
                "periodic_scan_frequency must be one of: every_15_minutes, every_30_minutes, \
                 hourly, every_2_hours, every_6_hours, every_12_hours, daily",
            ));
        }
    }
    if let Some(ref s) = patch.transcoder_hw_accel {
        if !matches!(
            s.as_str(),
            "auto" | "none" | "vaapi" | "nvenc" | "qsv" | "videotoolbox" | "amf"
        ) {
            return Err(ApiError::validation(
                "transcoder_hw_accel must be one of: auto, none, vaapi, nvenc, qsv, videotoolbox, amf",
            ));
        }
    }
    if let Some(ref s) = patch.transcoder_encoder_preset {
        if !matches!(s.as_str(), "speed" | "balanced" | "quality") {
            return Err(ApiError::validation(
                "transcoder_encoder_preset must be one of: speed, balanced, quality",
            ));
        }
    }
    if let Some(ref s) = patch.transcoder_hw_strictness {
        if !matches!(s.as_str(), "auto" | "prefer_hw" | "require_hw") {
            return Err(ApiError::validation(
                "transcoder_hw_strictness must be one of: auto, prefer_hw, require_hw",
            ));
        }
    }
    if let Some(ref s) = patch.transcoder_background_preset {
        if !matches!(
            s.as_str(),
            "ultrafast"
                | "superfast"
                | "veryfast"
                | "faster"
                | "fast"
                | "medium"
                | "slow"
                | "slower"
        ) {
            return Err(ApiError::validation(
                "transcoder_background_preset must be a libx264 preset name \
                 (ultrafast, superfast, veryfast, faster, fast, medium, slow, slower)",
            ));
        }
    }
    if let Some(n) = patch.transcoder_max_background_concurrent {
        if !(1..=16).contains(&n) {
            return Err(ApiError::validation(
                "transcoder_max_background_concurrent must be between 1 and 16",
            ));
        }
    }
    if let Some(n) = patch.job_workers {
        if !(1..=16).contains(&n) {
            return Err(ApiError::validation("job_workers must be between 1 and 16"));
        }
    }
    if let Some(raw) = patch.job_kind_concurrency.as_deref() {
        // Shape: JSON object whose values are positive integers in
        // [1, 32]. The cap of 32 matches the worker pool ceiling — a
        // per-kind cap higher than total workers is just confusing.
        let map: std::collections::HashMap<String, i64> =
            serde_json::from_str(raw).map_err(|e| {
                ApiError::validation(format!(
                    "job_kind_concurrency must be a JSON object mapping kind → integer: {e}"
                ))
            })?;
        for (k, v) in &map {
            if !(1..=32).contains(v) {
                return Err(ApiError::validation(format!(
                    "job_kind_concurrency[{k}] must be between 1 and 32 (got {v})"
                )));
            }
        }
    }
    if let Some(ref s) = patch.transcoder_hdr_tonemap_algo {
        if !matches!(
            s.as_str(),
            "hable" | "reinhard" | "mobius" | "bt2390" | "clip" | "linear"
        ) {
            return Err(ApiError::validation(
                "transcoder_hdr_tonemap_algo must be one of: hable, reinhard, mobius, bt2390, clip, linear",
            ));
        }
    }
    if let Some(n) = patch.transcoder_max_concurrent {
        if !(1..=64).contains(&n) {
            return Err(ApiError::validation(
                "transcoder_max_concurrent must be between 1 and 64",
            ));
        }
    }
    if let Some(Some(kbps)) = patch.transcoder_quality_ceiling_kbps {
        if !(100..=200_000).contains(&kbps) {
            return Err(ApiError::validation(
                "transcoder_quality_ceiling_kbps must be between 100 and 200000 when set",
            ));
        }
    }
    if let Some(ref raw) = patch.cors_origins {
        let parsed: serde_json::Result<Vec<String>> = serde_json::from_str(raw);
        match parsed {
            Ok(list) => {
                // Reject the wildcard entry. The CORS middleware also
                // refuses to honour `*` at request time (it would echo
                // an arbitrary origin with `Access-Control-Allow-
                // Credentials: true`, the classic credentialled-
                // wildcard misconfig). Catching it at write time gives
                // the operator an immediate error rather than a
                // mysterious "CORS just doesn't work" later.
                for entry in &list {
                    let trimmed = entry.trim();
                    if trimmed == "*" {
                        return Err(ApiError::validation(
                            "cors_origins cannot contain `*`. List explicit origins \
                             (e.g. https://flix.example.com) or leave the list empty \
                             to disable CORS entirely.",
                        ));
                    }
                    if !trimmed.is_empty()
                        && !trimmed.starts_with("http://")
                        && !trimmed.starts_with("https://")
                    {
                        return Err(ApiError::validation(format!(
                            "cors_origins entry `{trimmed}` must start with http:// or https://",
                        )));
                    }
                }
            }
            Err(_) => {
                return Err(ApiError::validation(
                    "cors_origins must be a JSON array of strings",
                ));
            }
        }
    }
    if let Some(ref raw) = patch.extras_json {
        let parsed: serde_json::Result<serde_json::Value> = serde_json::from_str(raw);
        if let Ok(v) = parsed {
            if !v.is_object() {
                return Err(ApiError::validation("extras_json must be a JSON object"));
            }
        } else {
            return Err(ApiError::validation("extras_json must be valid JSON"));
        }
    }
    if let Some(Some(ref url)) = patch.public_url {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ApiError::validation(
                "public_url must start with http:// or https://",
            ));
        }
    }
    if let Some(Some(ref s)) = patch.email_smtp_security {
        if !matches!(s.as_str(), "starttls" | "tls" | "none") {
            return Err(ApiError::validation(
                "email_smtp_security must be one of: starttls, tls, none",
            ));
        }
    }
    if let Some(Some(port)) = patch.email_smtp_port {
        if !(1..=65535).contains(&port) {
            return Err(ApiError::validation(
                "email_smtp_port must be between 1 and 65535",
            ));
        }
    }
    if let Some(Some(ref host)) = patch.email_smtp_host {
        if host.len() > 253 {
            return Err(ApiError::validation(
                "email_smtp_host must be at most 253 characters",
            ));
        }
        if host.contains(char::is_whitespace) {
            return Err(ApiError::validation(
                "email_smtp_host must not contain whitespace",
            ));
        }
    }
    if let Some(Some(ref user)) = patch.email_smtp_username {
        if user.len() > 256 {
            return Err(ApiError::validation(
                "email_smtp_username must be at most 256 characters",
            ));
        }
    }
    if let Some(Some(ref addr)) = patch.email_from_address {
        // Light-touch sanity check; lettre re-validates when building
        // the Mailbox. Reject obvious garbage early so the UI shows a
        // useful message instead of a generic SMTP connect failure.
        if !addr.contains('@') || addr.len() > 320 {
            return Err(ApiError::validation(
                "email_from_address must look like local@domain (max 320 chars)",
            ));
        }
    }
    if let Some(Some(ref name)) = patch.email_from_name {
        if name.len() > 128 {
            return Err(ApiError::validation(
                "email_from_name must be at most 128 characters",
            ));
        }
    }
    if let Some(ref s) = patch.totp_enforcement {
        if !matches!(s.as_str(), "disabled" | "optional" | "required") {
            return Err(ApiError::validation(
                "totp_enforcement must be one of: disabled, optional, required",
            ));
        }
    }
    if let Some(ref s) = patch.maintenance_window_start {
        validate_hhmm(s, "maintenance_window_start")?;
    }
    if let Some(ref s) = patch.maintenance_window_end {
        validate_hhmm(s, "maintenance_window_end")?;
    }
    if let Some(n) = patch.continue_watching_max_items {
        if !(1..=200).contains(&n) {
            return Err(ApiError::validation(
                "continue_watching_max_items must be between 1 and 200",
            ));
        }
    }
    if let Some(n) = patch.continue_watching_max_age_weeks {
        if !(0..=520).contains(&n) {
            return Err(ApiError::validation(
                "continue_watching_max_age_weeks must be between 0 (disable) and 520 (~10 years)",
            ));
        }
    }
    if let Some(n) = patch.video_played_threshold_pct {
        if !(50..=99).contains(&n) {
            return Err(ApiError::validation(
                "video_played_threshold_pct must be between 50 and 99",
            ));
        }
    }
    if let Some(n) = patch.database_cache_size_mb {
        if !(0..=4096).contains(&n) {
            return Err(ApiError::validation(
                "database_cache_size_mb must be between 0 (SQLite default) and 4096",
            ));
        }
    }
    // Network-fragment fields. Previously validated inline in
    // network.rs; folded in here so every PATCH handler that touches
    // server_settings runs the same checks.
    if let Some(n) = patch.transcoder_reaper_idle_threshold_ms {
        // 5s floor — anything lower and the 60s client keepalive plus
        // 15s reaper interval would race-kill healthy sessions. 1h
        // ceiling so a typo can't strand sessions for a day.
        if !(5_000..=3_600_000).contains(&n) {
            return Err(ApiError::validation(
                "transcoder_reaper_idle_threshold_ms must be between 5000 and 3600000",
            ));
        }
    }
    if let Some(n) = patch.max_remote_streams_per_user {
        if !(0..=64).contains(&n) {
            return Err(ApiError::validation(
                "max_remote_streams_per_user must be between 0 (unlimited) and 64",
            ));
        }
    }
    if let Some(ref raw) = patch.lan_networks {
        crate::net::validate_cidr_list(raw)
            .map_err(|e| ApiError::validation(format!("lan_networks: {e}")))?;
    }
    if let Some(ref raw) = patch.auth_bypass_cidrs {
        crate::net::validate_cidr_list(raw)
            .map_err(|e| ApiError::validation(format!("auth_bypass_cidrs: {e}")))?;
    }
    // Silence the unused `json!` import when downstream features go away.
    let _ = json!({});
    Ok(())
}

fn validate_hhmm(s: &str, field: &str) -> Result<(), ApiError> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return Err(ApiError::validation(format!(
            "{field} must be HH:MM (24-hour)"
        )));
    }
    let h: u32 = parts[0]
        .parse()
        .map_err(|_| ApiError::validation(format!("{field} hour must be 0-23")))?;
    let m: u32 = parts[1]
        .parse()
        .map_err(|_| ApiError::validation(format!("{field} minute must be 0-59")))?;
    if h >= 24 || m >= 60 {
        return Err(ApiError::validation(format!(
            "{field} must be HH:MM with hour 0-23 and minute 0-59"
        )));
    }
    Ok(())
}
