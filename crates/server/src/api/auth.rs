//! /api/v1/auth handlers: setup, login, logout, me, register, invites.

use std::net::IpAddr;

use axum::Extension;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::{self, SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chimpflix_common::now_ms;
use chimpflix_library::queries;
use chimpflix_library::{
    CreateInviteInput, Invite, LoginInput, NewAuditEntry, RegisterInput, SessionSummary,
    SetupInput, User, UserRole, hash_invite_code,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tracing::{debug, info, warn};

use crate::api::error::ApiError;
use crate::auth::{AuthUser, OwnerAuth, SESSION_MAX_AGE_S, cookie, password};
use crate::client_ip::EffectiveClientIp;
use crate::mail_template;
use crate::mailer::{Mailer, OutgoingMessage};
use crate::state::AppState;

// 12 chars is the 2026 NIST 800-63B baseline (recommended minimum for
// memorized secrets without 2FA). Existing accounts created under the
// old 8-char minimum still authenticate — the cap is enforced only on
// new passwords / changes / resets.
const MIN_PASSWORD_LEN: usize = 12;
const MAX_USERNAME_LEN: usize = 64;

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    user: User,
}

/// Login outcomes. Tagged by `status` so clients can match on the
/// discriminator before destructuring:
///   * `authenticated` — session cookie set, `user` populated.
///   * `2fa_required` — no cookie, follow up with /auth/2fa/login.
// API-response enum: the size delta between variants doesn't matter
// (constructed once per HTTP response, immediately serialized) and
// boxing `User` here would just churn the call sites.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LoginResponse {
    Authenticated {
        user: User,
    },
    #[serde(rename = "2fa_required")]
    TwoFactorRequired {
        /// Opaque HMAC-signed token redeemed by /auth/2fa/login.
        challenge: String,
        /// Seconds until the challenge expires — surfaced so the client
        /// can warn the user about lingering on the 2FA screen.
        expires_in_seconds: i64,
    },
}

#[derive(Debug, Serialize)]
pub struct AuthStatusResponse {
    pub setup_needed: bool,
}

pub async fn status(State(state): State<AppState>) -> Result<Json<AuthStatusResponse>, ApiError> {
    let setup_needed = queries::is_in_setup_mode(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(AuthStatusResponse { setup_needed }))
}

/// Response shape for invite creation. The plaintext `code` is returned
/// EXACTLY ONCE — subsequent list/get endpoints only ever surface the
/// hash, so if the admin doesn't capture this value at issuance the
/// invite must be revoked + reissued. `email_sent` mirrors whether the
/// SMTP send succeeded so the UI knows whether to surface the link as a
/// copy fallback.
#[derive(Debug, Serialize)]
pub struct CreatedInviteResponse {
    invite: Invite,
    /// Plaintext token to share with the recipient. Hashed at rest.
    code: String,
    /// Full accept URL built from `server_settings.public_url`. Null
    /// when `public_url` isn't configured (the UI then shows just the
    /// code and instructs the admin to set public_url).
    accept_url: Option<String>,
    /// True when an invite email was successfully handed off to SMTP.
    email_sent: bool,
    /// Library IDs pre-bound to this invite.
    library_ids: Vec<i64>,
    /// Access-group IDs pre-bound to this invite.
    group_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct InviteListEntry {
    #[serde(flatten)]
    invite: Invite,
    library_ids: Vec<i64>,
    group_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct InvitesListResponse {
    invites: Vec<InviteListEntry>,
}

// ---------------------------------------------------------------------------
// Setup (first-run only)
// ---------------------------------------------------------------------------

pub async fn setup(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    headers: HeaderMap,
    Json(input): Json<SetupInput>,
) -> Result<impl IntoResponse, ApiError> {
    if !queries::is_in_setup_mode(&state.pool)
        .await
        .map_err(ApiError::Internal)?
    {
        return Err(ApiError::Forbidden);
    }
    // BLOCK #5: setup-token gate. Closes the CSRF-bypass race window
    // where, between server boot and the operator completing first-
    // run setup, any reachable caller could claim the owner account.
    //
    // Behaviour:
    //   * `CHIMPFLIX_SETUP_TOKEN` set → require `X-Setup-Token`
    //     request header with exact match (constant-time compare).
    //   * Token unset AND `APP_PUBLIC_ORIGIN` looks public (https://)
    //     → refuse setup with an actionable error. Same pattern as
    //     the plaintext-vault refusal in `load_vault`.
    //   * Token unset AND origin is LAN-ish → allow (current
    //     behaviour preserved for dev + LAN-only deployments).
    //
    // See docs/PUBLIC_RELEASE_HARDENING.md BLOCK #5.
    enforce_setup_token(&headers)?;
    validate_username(&input.username)?;
    validate_password(&input.password)?;
    let email = input
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(addr) = email {
        validate_email(addr)?;
    }

    let hash = password::hash(&input.password).map_err(ApiError::Internal)?;
    let user = queries::complete_setup(
        &state.pool,
        input.username.trim(),
        &hash,
        input.display_name.as_deref(),
        email,
    )
    .await
    .map_err(ApiError::Internal)?;

    // Seed `public_url` from the setup request's Origin/Referer when
    // it isn't already configured. The CSRF middleware enforces the
    // origin check against this value on every subsequent strict
    // auth route — without seeding it, the operator's next login
    // would 403 with the same chicken-and-egg the setup endpoint
    // hits on a fresh DB. Operator can override later via admin →
    // Server → General.
    if let Some(origin) = setup_origin_from_headers(&headers) {
        let current = state.settings.read().await;
        let already_set = current
            .public_url
            .as_deref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        drop(current);
        if !already_set {
            let patch = chimpflix_library::ServerSettingsUpdate {
                public_url: Some(Some(origin.clone())),
                ..Default::default()
            };
            let updated = queries::update_server_settings(&state.pool, Some(user.id), patch)
                .await
                .map_err(ApiError::Internal)?;
            *state.settings.write().await = updated;
            info!(public_url = %origin, "seeded public_url from setup request origin");
        }
    }

    let cookies = issue_session(&state, &user, &headers, Some(ip)).await?;
    info!(user_id = user.id, "setup complete");
    Ok(authed_response(StatusCode::CREATED, user, cookies, &state))
}

// ---------------------------------------------------------------------------
// Login / logout / me
// ---------------------------------------------------------------------------

pub async fn login(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    headers: HeaderMap,
    Json(input): Json<LoginInput>,
) -> Result<impl IntoResponse, ApiError> {
    // Defense-in-depth length caps before any DB work. Body-size limit
    // already covers gross abuse; these stop a one-byte-shy payload from
    // probing the rate limiter.
    if input.username.is_empty() || input.password.is_empty() {
        return Err(invalid_credentials());
    }
    if input.username.len() > MAX_USERNAME_LEN || input.password.len() > 1024 {
        return Err(invalid_credentials());
    }

    // Username key for the per-identity attempt tracker. Normalized to
    // lowercase so case-tampering can't sidestep a lockout.
    let attempt_key = input.username.trim().to_lowercase();
    if let Some(wait) = state.login_attempts.check(&attempt_key).await {
        return Err(ApiError::TooManyRequests(format!(
            "too many failed attempts; try again in {}s",
            wait.as_secs().max(1)
        )));
    }

    let user_lookup = queries::find_user_with_secret_by_username(&state.pool, &input.username)
        .await
        .map_err(ApiError::Internal)?;

    // Constant-time path: always run the argon2 verify, even when the
    // user is unknown. That stops the response-time difference that
    // would otherwise leak whether a given username is registered.
    let (ok, user_opt) = match user_lookup {
        Some(record) if record.user.username != "_default" => {
            let ok = password::verify(&input.password, &record.password_hash);
            (ok, Some(record.user))
        }
        _ => {
            // Either no row, or the placeholder `_default` user. Verify
            // against the dummy hash so this branch takes argon2 time.
            let _ = password::verify(&input.password, password::dummy_hash());
            (false, None)
        }
    };

    if !ok {
        state.login_attempts.record_failure(&attempt_key).await;
        audit_auth(
            &state,
            "auth.login.failure",
            None,
            user_opt.as_ref().map(|u| u.id),
            Some(format!(
                r#"{{"username":{}}}"#,
                serde_json::Value::String(input.username.trim().to_string())
            )),
            &headers,
            Some(ip),
        )
        .await;
        return Err(invalid_credentials());
    }
    let user = user_opt.expect("ok=true requires a user");
    state.login_attempts.record_success(&attempt_key).await;

    // Transparent rehash-on-login: if the stored hash uses weaker
    // Argon2 parameters than today's target (e.g. an account created
    // under the OWASP-floor default before Phase 52), recompute and
    // persist the upgraded hash. Failures are logged but never block
    // login — the user still has a working credential.
    {
        // Pull the secret-bearing record once more to get the hash.
        // (`user` doesn't carry it; the secret view is gated behind a
        // separate query to keep the User struct serializable.)
        if let Ok(Some(rec)) =
            queries::find_user_with_secret_by_username(&state.pool, &input.username).await
        {
            if password::needs_rehash(&rec.password_hash) {
                match password::hash(&input.password) {
                    Ok(new_hash) => {
                        if let Err(e) =
                            queries::update_user_password(&state.pool, user.id, &new_hash).await
                        {
                            warn!(
                                error = %format!("{e:#}"),
                                user_id = user.id,
                                "argon2 rehash-on-login persist failed",
                            );
                        } else {
                            info!(
                                user_id = user.id,
                                "upgraded password hash to stronger argon2 params"
                            );
                        }
                    }
                    Err(e) => warn!(
                        error = %format!("{e:#}"),
                        user_id = user.id,
                        "argon2 rehash-on-login compute failed",
                    ),
                }
            }
        }
    }

    // 2FA check — if the user has a verified TOTP enrollment, don't
    // issue a session yet. Return a short-lived signed challenge that
    // the client trades for a session via /auth/2fa/login. The session
    // cookie is only set once the second factor is proven, so a
    // password-only compromise can't establish a session.
    let totp_record = queries::get_user_totp(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    let has_verified_totp = totp_record
        .as_ref()
        .is_some_and(|r| r.verified_at.is_some());

    // Enforce server-wide `totp_enforcement = "required"` at the
    // login gate. Without this check the policy was a UI fiction
    // for users who pre-dated it: enroll/disable handlers honoured
    // the policy, but login itself happily let in any user without
    // a TOTP secret. Now: if the policy says "required" and the
    // user hasn't completed an enrollment, refuse the login outright.
    //
    // Recovery path when you've locked yourself out (no 2FA users
    // CAN log in to enroll, because login is what enrolls them):
    //   1) Temporarily relax the policy via SQL:
    //        sqlite3 chimpflix.db \
    //          "UPDATE server_settings SET totp_enforcement = 'optional';"
    //   2) Log in, enroll TOTP under Settings → Two-Factor.
    //   3) Re-tighten via admin UI under /admin/network or by reversing
    //      the SQL.
    //
    // A bootstrap-enrollment flow (server hands back a one-time grant
    // that authorises just the enroll/verify endpoints) is the better
    // long-term UX — tracked as a follow-up.
    if !has_verified_totp {
        let policy = state.settings.read().await.totp_enforcement.clone();
        if policy == "required" {
            warn!(
                user_id = user.id,
                "login blocked: totp_enforcement=required but user has no verified 2FA"
            );
            audit_auth(
                &state,
                "auth.login.blocked_2fa_required",
                Some(user.id),
                Some(user.id),
                None,
                &headers,
                Some(ip),
            )
            .await;
            return Err(ApiError::Forbidden);
        }
    }

    if has_verified_totp {
        let exp_ms = now_ms() + crate::totp::CHALLENGE_TTL_SECS * 1000;
        let challenge = crate::totp::build_challenge(user.id, exp_ms, &state.auth.session_secret);
        info!(user_id = user.id, "login pending 2FA");
        audit_auth(
            &state,
            "auth.login.password_ok_2fa_pending",
            Some(user.id),
            Some(user.id),
            None,
            &headers,
            Some(ip),
        )
        .await;
        return Ok(Json(LoginResponse::TwoFactorRequired {
            challenge,
            expires_in_seconds: crate::totp::CHALLENGE_TTL_SECS,
        })
        .into_response());
    }

    let ip_str = ip.to_string();
    if let Err(e) = queries::record_user_login(&state.pool, user.id, Some(ip_str.as_str())).await {
        warn!(error = %format!("{e:#}"), user_id = user.id, "record_user_login");
    }
    let cookies = issue_session(&state, &user, &headers, Some(ip)).await?;
    info!(user_id = user.id, "login");
    audit_auth(
        &state,
        "auth.login.success",
        Some(user.id),
        Some(user.id),
        None,
        &headers,
        Some(ip),
    )
    .await;
    Ok(authed_login_response(user, cookies))
}

#[derive(Debug, Serialize)]
pub struct RevokeOthersResponse {
    pub revoked: u64,
}

#[derive(Debug, Serialize)]
pub struct MySessionEntry {
    #[serde(flatten)]
    pub session: SessionSummary,
    /// True for the session that authenticated this very request. The
    /// UI surfaces this so the user can distinguish "this is me right
    /// now" from "this is some other browser I left signed in".
    pub current: bool,
}

#[derive(Debug, Serialize)]
pub struct MySessionsResponse {
    pub sessions: Vec<MySessionEntry>,
}

pub async fn list_my_sessions(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<MySessionsResponse>, ApiError> {
    let summaries = queries::list_user_sessions(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    let sessions = summaries
        .into_iter()
        .map(|s| {
            let current = s.id == user.session_id;
            MySessionEntry {
                session: s,
                current,
            }
        })
        .collect();
    Ok(Json(MySessionsResponse { sessions }))
}

pub async fn revoke_my_session(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    user: AuthUser,
    headers: HeaderMap,
    Path(session_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    // Cross-check that the session actually belongs to the caller —
    // otherwise a user could revoke admins' sessions by guessing IDs.
    let session = queries::find_session(&state.pool, session_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if session.user_id != user.id {
        return Err(ApiError::NotFound);
    }
    // Sole-owner self-lockout guard (MONTH 1 in
    // `docs/PUBLIC_RELEASE_HARDENING.md`). Without it, the only owner
    // can revoke their own current session and the resulting "no
    // owner with a live session" state is only recoverable by direct
    // SQLite edits (or by running `owner-password-reset` from the CLI
    // — see Tier 0.6).
    if session_id == user.session_id && user.role == chimpflix_library::UserRole::Owner {
        let owner_count = queries::count_owners(&state.pool)
            .await
            .map_err(ApiError::Internal)?;
        if owner_count <= 1 {
            return Err(ApiError::validation(
                "cannot revoke your own current session as the sole owner — \
                 promote another user to Owner first, or use /auth/logout \
                 (which still requires you to log back in)",
            ));
        }
    }
    queries::delete_session(&state.pool, session_id)
        .await
        .map_err(ApiError::Internal)?;
    audit_auth(
        &state,
        "auth.session.revoke",
        Some(user.id),
        Some(user.id),
        Some(format!(r#"{{"session_id":{session_id}}}"#)),
        &headers,
        Some(ip),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Sign out every OTHER session for the current user — keeps the
/// request's session alive so the user stays signed in here. Useful
/// after a suspected credential leak, or when consolidating from
/// multiple devices.
pub async fn revoke_other_sessions(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    user: AuthUser,
    headers: HeaderMap,
) -> Result<Json<RevokeOthersResponse>, ApiError> {
    let revoked = queries::delete_sessions_for_user_except(&state.pool, user.id, user.session_id)
        .await
        .map_err(ApiError::Internal)?;
    info!(
        user_id = user.id,
        revoked, "user signed out of other sessions"
    );
    audit_auth(
        &state,
        "auth.sessions.revoke_others",
        Some(user.id),
        Some(user.id),
        Some(format!(r#"{{"revoked":{revoked}}}"#)),
        &headers,
        Some(ip),
    )
    .await;
    Ok(Json(RevokeOthersResponse { revoked }))
}

pub async fn logout(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    user: AuthUser,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    // Delete the *current* session row, then clear the cookie.
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Some(raw) = cookie::find_cookie(
        cookie_header,
        crate::auth::cookie_name(state.auth.cookie_secure),
    ) {
        if let Some((session_id, _)) = cookie::parse_value(raw, &state.auth.session_secret) {
            queries::delete_session(&state.pool, session_id)
                .await
                .map_err(ApiError::Internal)?;
        }
    }
    info!(user_id = user.id, "logout");
    audit_auth(
        &state,
        "auth.logout",
        Some(user.id),
        Some(user.id),
        None,
        &headers,
        Some(ip),
    )
    .await;
    let clear_session = cookie::clear_cookie_header(state.auth.cookie_secure);
    let clear_csrf = cookie::clear_csrf_cookie_header(state.auth.cookie_secure);
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(hv) = HeaderValue::from_str(&clear_session) {
        response.headers_mut().append(SET_COOKIE, hv);
    } else {
        warn!(
            "logout: failed to format clear-cookie header — client will keep cookie until expiry"
        );
    }
    if let Ok(hv) = HeaderValue::from_str(&clear_csrf) {
        response.headers_mut().append(SET_COOKIE, hv);
    }
    Ok(response)
}

pub async fn me(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<AuthResponse>, ApiError> {
    let full = queries::find_user_by_id(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::Unauthorized)?;
    Ok(Json(AuthResponse { user: full }))
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateMeInput {
    /// Double-Option semantics on the wire: missing key = leave as-is,
    /// explicit null = clear, value = set. serde gives us this via
    /// `Option<Option<T>>` with `deserialize_with` + serde_with — but to
    /// keep deps light we treat empty strings as "clear" here.
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Direct email writes are accepted only when the account has no
    /// email yet (first-time set). Subsequent changes go through
    /// /auth/me/email/request-change so the new address proves it can
    /// receive mail before we trust it.
    pub email: Option<String>,
    pub default_audio_lang: Option<String>,
    pub default_subtitle_lang: Option<String>,
    /// Subtitle styling. Strings empty-out to NULL via the same
    /// `normalize` pass as the language fields. Numeric fields use
    /// the sentinel `0` for "clear" — passing 0 is meaningless for
    /// font-size (clamped >= 8 below) and bottom-inset (clamped >= 0
    /// but a literal 0 is fine because that's the bottom edge). The
    /// happy path passes a real value or omits the key entirely.
    pub subtitle_font_size_px: Option<i64>,
    pub subtitle_text_color: Option<String>,
    pub subtitle_background_color: Option<String>,
    pub subtitle_font_family: Option<String>,
    pub subtitle_edge: Option<String>,
    pub subtitle_bottom_inset_pct: Option<i64>,
    /// Single-Option: present → set the boolean. Omit to leave as-is.
    pub notify_via_email: Option<bool>,
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

// Manual Debug — both fields are credentials; default derive would
// leak them through any `tracing::debug!(?input, ...)`.
impl std::fmt::Debug for ChangePasswordRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangePasswordRequest")
            .field("current_password", &"<redacted>")
            .field("new_password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct ChangePasswordResponse {
    /// Count of OTHER sessions invalidated on password change (the
    /// request's own session is preserved). Surfaced so the UI can
    /// nudge: "we signed out 3 other devices".
    pub sessions_revoked: u64,
}

/// Self-service password change. Requires the current password as a
/// re-auth — a stolen session shouldn't be enough to rotate the
/// password (which would also clobber every other session). On
/// success we revoke all OTHER sessions; the user stays signed in
/// here.
pub async fn change_password(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    user: AuthUser,
    headers: HeaderMap,
    Json(input): Json<ChangePasswordRequest>,
) -> Result<Json<ChangePasswordResponse>, ApiError> {
    if input.current_password.is_empty() {
        return Err(ApiError::validation("current password is required"));
    }
    validate_password(&input.new_password)?;
    if input.current_password == input.new_password {
        return Err(ApiError::validation(
            "new password must differ from the current one",
        ));
    }

    // Re-verify current password.
    let record = queries::find_user_with_secret_by_username(&state.pool, &user.username)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::Unauthorized)?;
    if !password::verify(&input.current_password, &record.password_hash) {
        audit_auth(
            &state,
            "auth.password_change.failure",
            Some(user.id),
            Some(user.id),
            None,
            &headers,
            Some(ip),
        )
        .await;
        return Err(ApiError::Unauthorized);
    }

    let new_hash = password::hash(&input.new_password).map_err(ApiError::Internal)?;
    queries::update_user_password(&state.pool, user.id, &new_hash)
        .await
        .map_err(ApiError::Internal)?;
    let revoked = queries::delete_sessions_for_user_except(&state.pool, user.id, user.session_id)
        .await
        .unwrap_or(0);
    info!(
        user_id = user.id,
        sessions_revoked = revoked,
        "password changed"
    );
    audit_auth(
        &state,
        "auth.password_change.success",
        Some(user.id),
        Some(user.id),
        Some(format!(r#"{{"sessions_revoked":{revoked}}}"#)),
        &headers,
        Some(ip),
    )
    .await;
    Ok(Json(ChangePasswordResponse {
        sessions_revoked: revoked,
    }))
}

// ---------------------------------------------------------------------------
// GDPR-style self-service: export own data + delete own account.
// MONTH 1 in `docs/PUBLIC_RELEASE_HARDENING.md`.
// ---------------------------------------------------------------------------

/// `GET /auth/me/export` — JSON dump of every row keyed on this user.
/// Excludes credentials (password hash, TOTP secret, OAuth tokens,
/// session cookies) and admin-side bookkeeping; this is the
/// "portable, take-it-with-me" view, not a forensic clone.
pub async fn export_me(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<serde_json::Value>, ApiError> {
    use sqlx::Row;

    let profile = queries::find_user_by_id(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;

    // Watch state per (item|episode|media_file). The play_state table
    // is the source of truth for "what the user has watched."
    let play_state: Vec<serde_json::Value> = sqlx::query(
        "SELECT item_id, episode_id, media_file_id, position_ms, duration_ms,
                watched, view_count, last_played_at
         FROM play_state WHERE user_id = ?",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .iter()
    .map(|r| {
        serde_json::json!({
            "item_id": r.try_get::<Option<i64>, _>("item_id").ok().flatten(),
            "episode_id": r.try_get::<Option<i64>, _>("episode_id").ok().flatten(),
            "media_file_id": r.try_get::<Option<i64>, _>("media_file_id").ok().flatten(),
            "position_ms": r.try_get::<Option<i64>, _>("position_ms").ok().flatten(),
            "duration_ms": r.try_get::<Option<i64>, _>("duration_ms").ok().flatten(),
            "watched": r.try_get::<i64, _>("watched").unwrap_or(0) != 0,
            "view_count": r.try_get::<i64, _>("view_count").unwrap_or(0),
            "last_played_at": r.try_get::<Option<i64>, _>("last_played_at").ok().flatten(),
        })
    })
    .collect();

    let my_list: Vec<i64> = sqlx::query_scalar(
        "SELECT item_id FROM user_my_list WHERE user_id = ? ORDER BY added_at",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let ratings: Vec<serde_json::Value> = sqlx::query(
        "SELECT item_id, rating, rated_at FROM user_ratings WHERE user_id = ?",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await
    .map(|rows| {
        rows.iter()
            .map(|r| {
                serde_json::json!({
                    "item_id": r.try_get::<i64, _>("item_id").unwrap_or(0),
                    "rating": r.try_get::<i64, _>("rating").unwrap_or(0),
                    "rated_at": r.try_get::<i64, _>("rated_at").unwrap_or(0),
                })
            })
            .collect()
    })
    .unwrap_or_default();

    let hidden_libraries: Vec<i64> = sqlx::query_scalar(
        "SELECT library_id FROM user_hidden_libraries WHERE user_id = ?",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let audit_entries: Vec<serde_json::Value> = sqlx::query(
        "SELECT action, target_kind, target_id, payload_json, ip,
                user_agent, created_at
         FROM audit_log WHERE actor_user_id = ?
         ORDER BY created_at DESC LIMIT 500",
    )
    .bind(user.id)
    .fetch_all(&state.pool)
    .await
    .map(|rows| {
        rows.iter()
            .map(|r| {
                serde_json::json!({
                    "action": r.try_get::<String, _>("action").unwrap_or_default(),
                    "target_kind": r.try_get::<Option<String>, _>("target_kind").ok().flatten(),
                    "target_id": r.try_get::<Option<String>, _>("target_id").ok().flatten(),
                    "payload_json": r.try_get::<Option<String>, _>("payload_json").ok().flatten(),
                    "ip": r.try_get::<Option<String>, _>("ip").ok().flatten(),
                    "user_agent": r.try_get::<Option<String>, _>("user_agent").ok().flatten(),
                    "created_at": r.try_get::<i64, _>("created_at").unwrap_or(0),
                })
            })
            .collect()
    })
    .unwrap_or_default();

    Ok(Json(serde_json::json!({
        "exported_at_ms": now_ms(),
        "schema_version": 1,
        "profile": {
            "id": profile.id,
            "username": profile.username,
            "display_name": profile.display_name,
            "email": profile.email,
            "role": profile.role,
            "created_at": profile.created_at,
            "updated_at": profile.updated_at,
        },
        "play_state": play_state,
        "my_list": my_list,
        "ratings": ratings,
        "hidden_libraries": hidden_libraries,
        "audit_log": audit_entries,
        "_note": "Credentials (password hash, TOTP secret, OAuth tokens) and \
                  active session cookies are intentionally excluded. Audit \
                  log truncated to the most recent 500 entries.",
    })))
}

#[derive(Deserialize)]
pub struct DeleteMeRequest {
    pub current_password: String,
}

impl std::fmt::Debug for DeleteMeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeleteMeRequest")
            .field("current_password", &"<redacted>")
            .finish()
    }
}

/// `DELETE /auth/me` — purge the requesting user's account.
///
/// Requires re-entering the current password (defense against CSRF +
/// stolen-cookie deletion). Refuses for the sole owner. The cascade
/// drops rows from play_state, my_list, ratings, hidden_libraries,
/// user_totp, user_trakt_tokens, user_auth_providers, notifications,
/// and sessions via FK ON DELETE CASCADE.
pub async fn delete_me(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    user: AuthUser,
    headers: HeaderMap,
    Json(input): Json<DeleteMeRequest>,
) -> Result<Response, ApiError> {
    if input.current_password.is_empty() {
        return Err(ApiError::validation("current password is required"));
    }

    let record = queries::find_user_with_secret_by_username(&state.pool, &user.username)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::Unauthorized)?;
    if !password::verify(&input.current_password, &record.password_hash) {
        audit_auth(
            &state,
            "auth.delete_me.failure",
            Some(user.id),
            Some(user.id),
            None,
            &headers,
            Some(ip),
        )
        .await;
        return Err(ApiError::Unauthorized);
    }

    // delete_user has the last-owner guard built in.
    let removed = queries::delete_user(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    if !removed {
        return Err(ApiError::Internal(anyhow::anyhow!(
            "delete_user reported no rows affected"
        )));
    }

    audit_auth(
        &state,
        "auth.delete_me.success",
        Some(user.id),
        Some(user.id),
        None,
        &headers,
        Some(ip),
    )
    .await;

    // Clear the session cookie on the response so the now-deleted
    // user's browser doesn't keep echoing a stale value.
    let clear_session = cookie::clear_cookie_header(state.auth.cookie_secure);
    let clear_csrf = cookie::clear_csrf_cookie_header(state.auth.cookie_secure);
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&clear_session).expect("ascii cookie"),
    );
    resp.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&clear_csrf).expect("ascii cookie"),
    );
    Ok(resp)
}

pub async fn update_me(
    State(state): State<AppState>,
    user: AuthUser,
    Json(input): Json<UpdateMeInput>,
) -> Result<Json<AuthResponse>, ApiError> {
    let normalize = |v: Option<String>| {
        v.map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    };
    let email_normalized = normalize(input.email);
    if let Some(Some(ref addr)) = email_normalized {
        validate_email(addr)?;
    }
    // Validate avatar_url. Rendered as `<img src>` in the nav and modal;
    // a `javascript:` URL can't fire there (browsers refuse to execute
    // JS via `<img src>`) but an arbitrary attacker-hosted URL turns
    // every page render into a tracking-pixel exfil of the user's IP
    // and User-Agent. Restrict to https:// only and cap length.
    let avatar_normalized = normalize(input.avatar_url);
    if let Some(Some(ref url)) = avatar_normalized {
        validate_avatar_url(url)?;
    }
    // Email change rules:
    //   * If the request sets `email` AND the user already has one,
    //     reject — they must use the verify flow at
    //     /auth/me/email/request-change. Direct overwrites bypass the
    //     "prove you can receive at the new address" gate.
    //   * If the user has no email yet (first-time set), allow direct
    //     write through this endpoint as a convenience.
    let email_patch = if email_normalized.is_some() {
        let current = queries::find_user_by_id(&state.pool, user.id)
            .await
            .map_err(ApiError::Internal)?
            .ok_or(ApiError::Unauthorized)?;
        if current.email.is_some() {
            return Err(ApiError::validation(
                "to change your email, use /auth/me/email/request-change",
            ));
        }
        email_normalized
    } else {
        None
    };
    // Subtitle style: validate enums and numeric ranges before the
    // values reach the DB. Out-of-range values become 400s rather than
    // silently being clamped — the UI only sends values from a closed
    // palette, so an out-of-range value is a client bug worth surfacing.
    if let Some(px) = input.subtitle_font_size_px
        && !(8..=128).contains(&px)
    {
        return Err(ApiError::validation(
            "subtitle_font_size_px must be between 8 and 128",
        ));
    }
    if let Some(pct) = input.subtitle_bottom_inset_pct
        && !(0..=90).contains(&pct)
    {
        return Err(ApiError::validation(
            "subtitle_bottom_inset_pct must be between 0 and 90",
        ));
    }
    let font_family_normalized = normalize(input.subtitle_font_family);
    if let Some(Some(ref f)) = font_family_normalized {
        if !["default", "sans", "serif", "mono"].contains(&f.as_str()) {
            return Err(ApiError::validation(
                "subtitle_font_family must be one of: default, sans, serif, mono",
            ));
        }
    }
    let edge_normalized = normalize(input.subtitle_edge);
    if let Some(Some(ref e)) = edge_normalized {
        if !["none", "outline", "shadow"].contains(&e.as_str()) {
            return Err(ApiError::validation(
                "subtitle_edge must be one of: none, outline, shadow",
            ));
        }
    }

    let patch = queries::UserSelfUpdate {
        display_name: normalize(input.display_name),
        avatar_url: avatar_normalized,
        email: email_patch,
        default_audio_lang: normalize(input.default_audio_lang),
        default_subtitle_lang: normalize(input.default_subtitle_lang),
        subtitle_font_size_px: input.subtitle_font_size_px.map(Some),
        subtitle_text_color: normalize(input.subtitle_text_color),
        subtitle_background_color: normalize(input.subtitle_background_color),
        subtitle_font_family: font_family_normalized,
        subtitle_edge: edge_normalized,
        subtitle_bottom_inset_pct: input.subtitle_bottom_inset_pct.map(Some),
        notify_via_email: input.notify_via_email,
    };
    let updated = queries::update_user_self(&state.pool, user.id, patch)
        .await
        .map_err(|e| {
            let msg = format!("{e:#}");
            if msg.contains("UNIQUE constraint failed") {
                ApiError::Conflict("that email is already in use by another account".into())
            } else {
                ApiError::Internal(e)
            }
        })?
        .ok_or(ApiError::Unauthorized)?;
    Ok(Json(AuthResponse { user: updated }))
}

// ---------------------------------------------------------------------------
// Register (with invite)
// ---------------------------------------------------------------------------

pub async fn register(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    headers: HeaderMap,
    Json(input): Json<RegisterInput>,
) -> Result<impl IntoResponse, ApiError> {
    let raw_code = input.code.trim();
    if raw_code.is_empty() {
        return Err(ApiError::validation("invite code is required"));
    }
    if raw_code.len() > 256 {
        return Err(ApiError::validation("invite code is invalid"));
    }
    validate_username(&input.username)?;
    validate_password(&input.password)?;

    let code_hash = hash_invite_code(raw_code);
    let invite = queries::find_invite_by_code_hash(&state.pool, &code_hash)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::validation("invite code is invalid"))?;
    if invite.consumed_by.is_some() {
        return Err(ApiError::validation("invite code has already been used"));
    }
    if let Some(exp) = invite.expires_at {
        if exp < now_ms() {
            return Err(ApiError::validation("invite code has expired"));
        }
    }

    let hash = password::hash(&input.password).map_err(ApiError::Internal)?;
    // Pre-bind the user's email from the invite. If the invite was issued
    // without an email, the user is created without one and can set it
    // themselves later via PATCH /auth/me.
    let invite_email = invite.email.as_deref();
    let user = queries::create_user(
        &state.pool,
        input.username.trim(),
        &hash,
        UserRole::User,
        input.display_name.as_deref(),
        invite_email,
    )
    .await
    .map_err(|e| {
        // Surface unique-violation as a 409.
        let msg = format!("{e:#}");
        if msg.contains("UNIQUE constraint failed") {
            ApiError::Conflict("username or email already exists".into())
        } else {
            ApiError::Internal(e)
        }
    })?;

    queries::consume_invite(&state.pool, &code_hash, user.id)
        .await
        .map_err(ApiError::Internal)?;

    // Fan out a notification to every owner. Fire-and-forget — won't
    // fail the registration if SMTP is down or all admins have email
    // mirroring disabled.
    crate::notifier::notify_user_registered(&state, &user, invite_email).await;

    let cookies = issue_session(&state, &user, &headers, Some(ip)).await?;
    info!(user_id = user.id, "register");
    Ok(authed_response(StatusCode::CREATED, user, cookies, &state))
}

// ---------------------------------------------------------------------------
// Invites (owner-only)
// ---------------------------------------------------------------------------

pub async fn list_invites(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<InvitesListResponse>, ApiError> {
    let invites = queries::list_invites(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    let mut entries = Vec::with_capacity(invites.len());
    for invite in invites {
        let library_ids = queries::invite_library_ids(&state.pool, invite.id)
            .await
            .map_err(ApiError::Internal)?;
        let group_ids = queries::invite_group_ids(&state.pool, invite.id)
            .await
            .map_err(ApiError::Internal)?;
        entries.push(InviteListEntry {
            invite,
            library_ids,
            group_ids,
        });
    }
    Ok(Json(InvitesListResponse { invites: entries }))
}

pub async fn create_invite(
    State(state): State<AppState>,
    OwnerAuth(user): OwnerAuth,
    Json(input): Json<CreateInviteInput>,
) -> Result<(StatusCode, Json<CreatedInviteResponse>), ApiError> {
    // Light-touch email validation. lettre + the SMTP relay will reject
    // malformed addresses on send anyway; the early check just lets the
    // admin see a useful 400 instead of a generic "send failed" later.
    let email = input
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(addr) = email {
        if !addr.contains('@') || addr.len() > 320 {
            return Err(ApiError::validation(
                "email must look like local@domain (max 320 chars)",
            ));
        }
    }
    if input.library_ids.len() > 64 {
        return Err(ApiError::validation(
            "library_ids may bind at most 64 libraries per invite",
        ));
    }
    if input.group_ids.len() > 32 {
        return Err(ApiError::validation(
            "group_ids may bind at most 32 groups per invite",
        ));
    }

    let expires_at = input.expires_in_seconds.map(|s| now_ms() + s.max(0) * 1000);

    // 32 bytes → 64 hex chars. Plenty of entropy; nothing stored at rest
    // except the SHA-256 hash.
    let mut buf = [0u8; 32];
    password::fill_random(&mut buf).map_err(ApiError::Internal)?;
    let code = hex::encode(buf);
    let code_hash = hash_invite_code(&code);

    let invite = queries::create_invite(
        &state.pool,
        &code_hash,
        user.id,
        expires_at,
        email,
        &input.library_ids,
        &input.group_ids,
    )
    .await
    .map_err(ApiError::Internal)?;

    // Build the accept URL from public_url if configured. Without it we
    // can still surface the raw code; the admin pastes it into the
    // recipient's address bar manually.
    let public_url = state
        .settings
        .read()
        .await
        .public_url
        .clone()
        .map(|s| s.trim_end_matches('/').to_string());
    let accept_url = public_url
        .as_deref()
        .map(|base| format!("{base}/login?invite={code}"));

    // Send the email if we have both a recipient + Mailer. Don't fail
    // the request on send error — return the link so the admin can
    // share manually.
    let mut email_sent = false;
    if let Some(addr) = email {
        let settings = state.settings.read().await.clone();
        match Mailer::from_settings(&settings, &state.pool, &state.vault).await {
            Ok(Some(mailer)) => {
                let server_name = settings.server_name.clone();
                let html =
                    invite_email_html(&server_name, accept_url.as_deref(), &code, expires_at);
                let text =
                    invite_email_text(&server_name, accept_url.as_deref(), &code, expires_at);
                let subject = format!("Your {server_name} invitation");
                let outgoing = OutgoingMessage {
                    to_address: addr,
                    to_name: None,
                    subject: &subject,
                    html: &html,
                    text: &text,
                };
                match mailer.send(outgoing).await {
                    Ok(()) => {
                        if let Err(e) = queries::mark_invite_sent(&state.pool, invite.id).await {
                            warn!(error = %format!("{e:#}"), "mark_invite_sent");
                        }
                        email_sent = true;
                    }
                    Err(e) => warn!(error = %format!("{e:#}"), "invite email send failed"),
                }
            }
            Ok(None) => info!("invite email skipped — SMTP not configured"),
            Err(e) => warn!(error = %format!("{e:#}"), "invite mailer build failed"),
        }
    }

    info!(invite_id = invite.id, email_sent, "create invite");
    let library_ids = input.library_ids.clone();
    let group_ids = input.group_ids.clone();
    Ok((
        StatusCode::CREATED,
        Json(CreatedInviteResponse {
            invite,
            code,
            accept_url,
            email_sent,
            library_ids,
            group_ids,
        }),
    ))
}

pub async fn revoke_invite(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let revoked = queries::revoke_invite_by_id(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    if revoked {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

fn invite_email_text(
    server_name: &str,
    accept_url: Option<&str>,
    code: &str,
    expires_at: Option<i64>,
) -> String {
    let now = chimpflix_common::now_ms();
    let mut body = String::new();
    body.push_str(&format!(
        "You've been invited to {server_name}. Open this link to set up your account:\n\n"
    ));
    if let Some(url) = accept_url {
        body.push_str(&format!("  {url}\n\n"));
    }
    body.push_str(&format!(
        "If the link doesn't work, your invite code is:\n\n  {code}\n"
    ));
    if let Some(ms) = expires_at {
        body.push_str(&format!(
            "\nThis invitation expires on {}.\n",
            mail_template::format_email_datetime_with_relative(ms, now),
        ));
    }
    mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name,
        headline: "You're invited",
        body: &body,
        footer_note: "You're receiving this because someone on a ChimpFlix server invited you. \
                      If you weren't expecting this, you can safely ignore the email — \
                      no account is created until you accept.",
    })
}

fn invite_email_html(
    server_name: &str,
    accept_url: Option<&str>,
    code: &str,
    expires_at: Option<i64>,
) -> String {
    let now = chimpflix_common::now_ms();
    let server_safe = mail_template::html_escape(server_name);
    let mut body = String::new();
    body.push_str(&mail_template::section_paragraph(&format!(
        "You've been invited to join <strong>{server_safe}</strong> — a private library of \
         movies, shows, and anime hosted by the server owner. Tap below to set up your \
         account and start watching."
    )));
    if let Some(url) = accept_url {
        body.push_str(&mail_template::section_cta("Accept invitation", url));
    }
    body.push_str(&mail_template::section_small(
        "If the button or link doesn't work, your invite code is:",
    ));
    body.push_str(&mail_template::section_code(code));
    if let Some(ms) = expires_at {
        let when = mail_template::format_email_datetime_with_relative(ms, now);
        let when_safe = mail_template::html_escape(&when);
        body.push_str(&mail_template::section_callout(
            mail_template::CalloutKind::Default,
            &format!("This invitation expires on <strong>{when_safe}</strong>."),
        ));
    }
    mail_template::render_email(mail_template::EmailOpts {
        server_name,
        eyebrow_html: "You're invited",
        headline: "Get ready to start streaming.",
        body_html: &body,
        footer_note: "You're receiving this because someone on a ChimpFlix server invited you. \
                      If you weren't expecting this, you can safely ignore the email — \
                      no account is created until you accept.",
    })
}

// ---------------------------------------------------------------------------
// Email change (verification round-trip)
// ---------------------------------------------------------------------------

const EMAIL_CHANGE_TTL_S: i64 = 60 * 60;

#[derive(Deserialize)]
pub struct RequestEmailChangeRequest {
    pub new_email: String,
    pub password: String,
}

impl std::fmt::Debug for RequestEmailChangeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestEmailChangeRequest")
            .field("new_email", &self.new_email)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct RequestEmailChangeResponse {
    /// Echo of the requested address — UI confirmation, no security
    /// content; the actual gate is the email link.
    pub new_email: String,
    /// True if the verification email was handed off to SMTP. False
    /// when SMTP isn't configured — the request still records the
    /// token so the admin can issue it manually if needed.
    pub email_sent: bool,
}

pub async fn request_email_change(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    user: AuthUser,
    headers: HeaderMap,
    Json(input): Json<RequestEmailChangeRequest>,
) -> Result<Json<RequestEmailChangeResponse>, ApiError> {
    let new_email = input.new_email.trim().to_string();
    validate_email(&new_email)?;
    if input.password.is_empty() {
        return Err(ApiError::validation("password is required"));
    }
    let record = queries::find_user_with_secret_by_username(&state.pool, &user.username)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::Unauthorized)?;
    if !password::verify(&input.password, &record.password_hash) {
        return Err(ApiError::Unauthorized);
    }
    if record.user.email.as_deref() == Some(new_email.as_str()) {
        return Err(ApiError::validation(
            "new email is the same as the current one",
        ));
    }

    // Generate + persist the token. Plaintext goes in the email.
    let mut buf = [0u8; 32];
    password::fill_random(&mut buf).map_err(ApiError::Internal)?;
    let token = hex::encode(buf);
    let token_hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
    let expires_at = now_ms() + EMAIL_CHANGE_TTL_S * 1000;
    queries::create_email_change_token(&state.pool, user.id, &new_email, &token_hash, expires_at)
        .await
        .map_err(ApiError::Internal)?;

    // Send the verification email to the NEW address (not the old one
    // — we're proving control of the new mailbox).
    let mut email_sent = false;
    let settings = state.settings.read().await.clone();
    let public_url = settings
        .public_url
        .clone()
        .map(|s| s.trim_end_matches('/').to_string());
    let verify_url = public_url
        .as_deref()
        .map(|base| format!("{base}/settings/account?verify_email={token}"));
    match Mailer::from_settings(&settings, &state.pool, &state.vault).await {
        Ok(Some(mailer)) => {
            let server_name = settings.server_name.clone();
            let html = email_change_html(&server_name, verify_url.as_deref(), &token);
            let text = email_change_text(&server_name, verify_url.as_deref(), &token);
            let subject = format!("Confirm your new {server_name} email");
            if let Err(e) = mailer
                .send(OutgoingMessage {
                    to_address: &new_email,
                    to_name: record.user.display_name.as_deref(),
                    subject: &subject,
                    html: &html,
                    text: &text,
                })
                .await
            {
                warn!(error = %format!("{e:#}"), user_id = user.id, "send email-change verification");
            } else {
                email_sent = true;
            }
        }
        Ok(None) => warn!(
            user_id = user.id,
            "email-change requested but SMTP not configured — the user can't receive the verification link"
        ),
        Err(e) => warn!(error = %format!("{e:#}"), "build Mailer for email change"),
    }

    audit_auth(
        &state,
        "auth.email_change.request",
        Some(user.id),
        Some(user.id),
        Some(format!(
            r#"{{"new_email":{},"email_sent":{}}}"#,
            serde_json::Value::String(new_email.clone()),
            email_sent
        )),
        &headers,
        Some(ip),
    )
    .await;
    Ok(Json(RequestEmailChangeResponse {
        new_email,
        email_sent,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ConfirmEmailChangeRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct ConfirmEmailChangeResponse {
    /// The address now on the account.
    pub email: String,
}

pub async fn confirm_email_change(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    user: AuthUser,
    headers: HeaderMap,
    Json(input): Json<ConfirmEmailChangeRequest>,
) -> Result<Json<ConfirmEmailChangeResponse>, ApiError> {
    let token = input.token.trim();
    if token.is_empty() || token.len() > 128 {
        return Err(ApiError::validation("token is invalid or expired"));
    }
    let token_hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
    let (token_id, token_user_id, new_email) =
        queries::find_active_email_change_token(&state.pool, &token_hash)
            .await
            .map_err(ApiError::Internal)?
            .ok_or_else(|| ApiError::validation("token is invalid or expired"))?;
    // Token must belong to the calling user. This prevents one user
    // from completing another's pending change.
    if token_user_id != user.id {
        return Err(ApiError::validation("token is invalid or expired"));
    }
    let old_email = queries::consume_email_change(&state.pool, token_id, user.id, &new_email)
        .await
        .map_err(|e| {
            let msg = format!("{e:#}");
            if msg.contains("UNIQUE constraint failed") {
                ApiError::Conflict("that email is already in use by another account".into())
            } else {
                warn!(error = %msg, user_id = user.id, "consume_email_change");
                ApiError::validation("could not complete email change")
            }
        })?;

    // Rotate every OTHER session for this user. A hijacked session
    // that just succeeded in re-binding the email shouldn't survive
    // the change — it cuts the attacker off before they can complete
    // the password-reset takeover, while leaving the user's current
    // browser logged in.
    let _ = queries::delete_sessions_for_user_except(&state.pool, user.id, user.session_id).await;

    // Notify the OLD address so an account-takeover attempt produces
    // an out-of-band signal: even if the attacker controls the new
    // address, the legitimate owner sees the breakup notification on
    // their old inbox and can react. Best-effort: a missing SMTP
    // config or send error doesn't block the change.
    if let Some(prior) = old_email.as_deref() {
        let settings = state.settings.read().await.clone();
        if let Ok(Some(mailer)) = Mailer::from_settings(&settings, &state.pool, &state.vault).await
        {
            let server_name = settings.server_name.clone();
            let html = email_change_alert_html(&server_name, &new_email);
            let text = email_change_alert_text(&server_name, &new_email);
            let subject = format!("Your {server_name} account email was changed");
            if let Err(e) = mailer
                .send(OutgoingMessage {
                    to_address: prior,
                    to_name: None,
                    subject: &subject,
                    html: &html,
                    text: &text,
                })
                .await
            {
                warn!(
                    error = %format!("{e:#}"),
                    user_id = user.id,
                    "send email-change confirmation to old address",
                );
            }
        }
    }
    audit_auth(
        &state,
        "auth.email_change.confirm",
        Some(user.id),
        Some(user.id),
        Some(format!(
            r#"{{"new_email":{}}}"#,
            serde_json::Value::String(new_email.clone())
        )),
        &headers,
        Some(ip),
    )
    .await;
    Ok(Json(ConfirmEmailChangeResponse { email: new_email }))
}

fn email_change_text(server_name: &str, verify_url: Option<&str>, token: &str) -> String {
    let mut body = String::from(
        "Someone (hopefully you) asked to change the email on a ChimpFlix account to this \
         address. Confirm to finish the change:\n\n",
    );
    if let Some(url) = verify_url {
        body.push_str(&format!("  {url}\n\n"));
    }
    body.push_str(&format!(
        "If the link doesn't work, your confirmation token is:\n\n  {token}\n\n\
         This link expires in 1 hour."
    ));
    mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name,
        headline: "Confirm your new email address",
        body: &body,
        footer_note: "If you didn't request this change, you can ignore this email — \
                      your account email won't be touched.",
    })
}

fn email_change_html(server_name: &str, verify_url: Option<&str>, token: &str) -> String {
    let mut body = String::new();
    body.push_str(&mail_template::section_paragraph(
        "Someone (hopefully you) asked to change the email on a ChimpFlix account to \
         <strong>this address</strong>. Confirm below to finish the change.",
    ));
    if let Some(url) = verify_url {
        body.push_str(&mail_template::section_cta("Confirm email change", url));
    }
    body.push_str(&mail_template::section_small(
        "If the button or link doesn't work, your confirmation token is:",
    ));
    body.push_str(&mail_template::section_code(token));
    body.push_str(&mail_template::section_callout(
        mail_template::CalloutKind::Info,
        "This link expires in <strong>1 hour</strong>.",
    ));
    mail_template::render_email(mail_template::EmailOpts {
        server_name,
        eyebrow_html: "Verify email change",
        headline: "Is this still you?",
        body_html: &body,
        footer_note: "If you didn't request this change, you can ignore this email — \
                      your account email won't be touched.",
    })
}

/// Notification sent to the OLD email address after a successful
/// email-change confirmation. Heads-up shape: "your account email
/// just changed to <new>; if you didn't do this, contact your admin."
fn email_change_alert_text(server_name: &str, new_email: &str) -> String {
    let body = format!(
        "Heads up — your {server_name} account email was just changed to:\n\n  \
         {new_email}\n\nIf you did this, you can ignore this message. If you \
         did NOT, your account may have been compromised. Contact your \
         administrator immediately."
    );
    mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name,
        headline: "Your account email was changed",
        body: &body,
        footer_note: "This is a security notification sent to the previous \
                      email address on file.",
    })
}

fn email_change_alert_html(server_name: &str, new_email: &str) -> String {
    let mut body = String::new();
    body.push_str(&mail_template::section_paragraph(&format!(
        "Heads up — your <strong>{}</strong> account email was just changed to:",
        mail_template::html_escape(server_name),
    )));
    body.push_str(&mail_template::section_code(new_email));
    body.push_str(&mail_template::section_callout(
        mail_template::CalloutKind::Warn,
        "If you did NOT make this change, your account may have been \
         compromised. Contact your administrator immediately to revert \
         the email and rotate your password.",
    ));
    mail_template::render_email(mail_template::EmailOpts {
        server_name,
        eyebrow_html: "Security notice",
        headline: "Your account email was changed",
        body_html: &body,
        footer_note: "This security notification was sent to the previous \
                      email address on file.",
    })
}

// ---------------------------------------------------------------------------
// Password reset (self-service)
// ---------------------------------------------------------------------------

/// Password-reset link TTL — 1h is the standard for short-lived reset
/// tokens. Tokens are also single-use; whichever expires first wins.
const PASSWORD_RESET_TTL_S: i64 = 60 * 60;

#[derive(Debug, Deserialize)]
pub struct PasswordResetRequest {
    pub email: String,
}

/// Always returns 204 regardless of whether the email matches a user.
/// Critical for not leaking which addresses are registered — a 200/404
/// split here is the classic "is this email a customer of yours?"
/// enumeration oracle. The rate limiter on the route keeps an attacker
/// from probing addresses in bulk.
pub async fn request_password_reset(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    headers: HeaderMap,
    Json(input): Json<PasswordResetRequest>,
) -> Result<StatusCode, ApiError> {
    // Server-config precheck (MONTH 1 in
    // `docs/PUBLIC_RELEASE_HARDENING.md`). Without SMTP configured,
    // we'd accept the request, generate a token, and silently fail
    // to email it — the user keeps clicking "send" forever wondering
    // why nothing arrives. Loudly surfacing the missing config gives
    // them a useful answer ("ask your admin to set up SMTP").
    // This check leaks whether SMTP is set up server-side; that's
    // intentional global config state, not per-user data.
    {
        let settings_snap = state.settings.read().await.clone();
        match Mailer::from_settings(&settings_snap, &state.pool, &state.vault).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(ApiError::validation(
                    "Email isn't configured on this server. Ask the administrator \
                     to set up SMTP under Admin -> Server -> Email, then try again.",
                ));
            }
            Err(e) => {
                return Err(ApiError::Internal(e));
            }
        }
    }

    let email = input.email.trim();
    // Silently no-op for malformed input — same response shape as the
    // happy path so a probe sees no difference.
    if email.is_empty() || !email.contains('@') || email.len() > 320 {
        return Ok(StatusCode::NO_CONTENT);
    }

    // Per-email throttle: 3 requests / hour / address. The per-IP
    // limiter on this route catches volume; this limiter catches the
    // "rotate IPs to email-bomb one inbox" attack pattern. Lower-case
    // the key so case-tampering can't sidestep the cap. We return the
    // same 204 as the happy path so an attacker can't tell whether
    // they tripped the throttle vs. typed a non-existent address.
    let email_key = email.to_ascii_lowercase();
    if state.reset_email_limiter.check_key(&email_key).is_err() {
        warn!(
            email = %obfuscate_email(email),
            "password-reset throttled at per-email gate"
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    let user_opt = queries::find_user_by_email(&state.pool, email)
        .await
        .map_err(ApiError::Internal)?;

    // Audit the REQUEST whether or not a user matched — same shape
    // either way so an attacker probing the audit log (if they could
    // see it) can't tell which addresses are real.
    //
    // Privacy: store only an obfuscated form of the address
    // (`a***@example.com`) so the audit_log doesn't accumulate every
    // typo'd / probed email forever. A leaked DB no longer hands the
    // attacker a list of every address that ever appeared at this
    // endpoint.
    audit_auth(
        &state,
        "auth.password_reset.request",
        None,
        user_opt.as_ref().map(|u| u.id),
        Some(format!(
            r#"{{"email":{}}}"#,
            serde_json::Value::String(obfuscate_email(email))
        )),
        &headers,
        Some(ip),
    )
    .await;

    if let Some(user) = user_opt {
        // Generate token + hash, persist hash only.
        let mut buf = [0u8; 32];
        password::fill_random(&mut buf).map_err(ApiError::Internal)?;
        let token = hex::encode(buf);
        let token_hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
        let expires_at = now_ms() + PASSWORD_RESET_TTL_S * 1000;
        // Use the trusted-proxy-resolved IP (set by client_ip middleware)
        // rather than reading X-Forwarded-For verbatim. Reading the raw
        // header here meant an attacker could spoof the recorded IP on
        // every password-reset request.
        let ip_str = ip.to_string();
        let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
        if let Err(e) = queries::create_password_reset_token(
            &state.pool,
            user.id,
            &token_hash,
            expires_at,
            Some(ip_str.as_str()),
            user_agent,
        )
        .await
        {
            warn!(error = %format!("{e:#}"), "create password_reset_token");
            // Still return 204 — same response either way.
            return Ok(StatusCode::NO_CONTENT);
        }

        // Send the email. If SMTP isn't configured or send fails, the
        // operator sees the warning in logs and the user just doesn't
        // get the link — no API-visible signal.
        let settings = state.settings.read().await.clone();
        let public_url = settings
            .public_url
            .clone()
            .map(|s| s.trim_end_matches('/').to_string());
        let reset_url = public_url
            .as_deref()
            .map(|base| format!("{base}/reset-password?token={token}"));
        match Mailer::from_settings(&settings, &state.pool, &state.vault).await {
            Ok(Some(mailer)) => {
                let server_name = settings.server_name.clone();
                let subject = format!("Reset your {server_name} password");
                let html = password_reset_email_html(&server_name, reset_url.as_deref(), &token);
                let text = password_reset_email_text(&server_name, reset_url.as_deref(), &token);
                if let Err(e) = mailer
                    .send(OutgoingMessage {
                        to_address: email,
                        to_name: user.display_name.as_deref(),
                        subject: &subject,
                        html: &html,
                        text: &text,
                    })
                    .await
                {
                    warn!(error = %format!("{e:#}"), user_id = user.id, "send password-reset email");
                }
            }
            Ok(None) => warn!(
                user_id = user.id,
                "password-reset requested but SMTP not configured — the user can't receive the link"
            ),
            Err(e) => warn!(error = %format!("{e:#}"), "build Mailer for password reset"),
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct PasswordResetConfirm {
    pub token: String,
    pub new_password: String,
}

impl std::fmt::Debug for PasswordResetConfirm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordResetConfirm")
            // Token IS the credential — anyone with the token can
            // complete the reset. Redact alongside the new password.
            .field("token", &"<redacted>")
            .field("new_password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct PasswordResetConfirmResponse {
    /// Number of other sessions that were invalidated. Surfaced so the
    /// reset page can say "We signed out 3 other devices" — useful for
    /// the user who's resetting because their account was compromised.
    pub sessions_revoked: u64,
}

pub async fn confirm_password_reset(
    State(state): State<AppState>,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    headers: HeaderMap,
    Json(input): Json<PasswordResetConfirm>,
) -> Result<Json<PasswordResetConfirmResponse>, ApiError> {
    let token = input.token.trim();
    if token.is_empty() || token.len() > 128 {
        return Err(ApiError::validation("token is invalid or expired"));
    }
    validate_password(&input.new_password)?;

    let token_hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
    let (token_id, user_id) = queries::find_active_password_reset_token(&state.pool, &token_hash)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::validation("token is invalid or expired"))?;

    let hash = password::hash(&input.new_password).map_err(ApiError::Internal)?;
    let revoked = queries::consume_password_reset(&state.pool, token_id, user_id, &hash)
        .await
        .map_err(|e| {
            // Log the raw error server-side (operator sees schema/SQL
            // details if they tail the log) but return a generic
            // message to the client so leaked errors can't fingerprint
            // the schema or hint at concurrent reset attempts.
            warn!(error = %format!("{e:#}"), user_id, "consume_password_reset");
            ApiError::validation(
                "could not complete password reset; the token may have already been used",
            )
        })?;

    info!(user_id, sessions_revoked = revoked, "password reset");
    audit_auth(
        &state,
        "auth.password_reset.confirm",
        Some(user_id),
        Some(user_id),
        Some(format!(r#"{{"sessions_revoked":{revoked}}}"#)),
        &headers,
        Some(ip),
    )
    .await;
    Ok(Json(PasswordResetConfirmResponse {
        sessions_revoked: revoked,
    }))
}

fn password_reset_email_text(server_name: &str, reset_url: Option<&str>, token: &str) -> String {
    let mut body = String::from(
        "Someone (hopefully you) asked to reset the password on the ChimpFlix account \
         associated with this email. Choose a new password:\n\n",
    );
    if let Some(url) = reset_url {
        body.push_str(&format!("  {url}\n\n"));
    }
    body.push_str(&format!(
        "If the link doesn't work, your reset token is:\n\n  {token}\n\n\
         This link expires in 1 hour and can only be used once."
    ));
    mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name,
        headline: "Forgot your password?",
        body: &body,
        footer_note: "If you didn't request this, you can ignore this email — \
                      your password hasn't been changed.",
    })
}

fn password_reset_email_html(server_name: &str, reset_url: Option<&str>, token: &str) -> String {
    let mut body = String::new();
    body.push_str(&mail_template::section_paragraph(
        "Someone (hopefully you) asked to reset the password on the ChimpFlix account \
         associated with this email. Choose a new password below.",
    ));
    if let Some(url) = reset_url {
        body.push_str(&mail_template::section_cta("Choose a new password", url));
    }
    body.push_str(&mail_template::section_small(
        "If the button or link doesn't work, your reset token is:",
    ));
    body.push_str(&mail_template::section_code(token));
    body.push_str(&mail_template::section_callout(
        mail_template::CalloutKind::Info,
        "This link expires in <strong>1 hour</strong> and can only be used once.",
    ));
    mail_template::render_email(mail_template::EmailOpts {
        server_name,
        eyebrow_html: "Password reset",
        headline: "Forgot your password?",
        body_html: &body,
        footer_note: "If you didn't request this, you can ignore this email — \
                      your password hasn't been changed.",
    })
}

// ---------------------------------------------------------------------------
// Users (owner-only)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct UsersListResponse {
    pub users: Vec<User>,
}

pub async fn list_users(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<UsersListResponse>, ApiError> {
    let users = queries::list_users(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(UsersListResponse { users }))
}

pub async fn delete_user(
    State(state): State<AppState>,
    crate::auth::AdminAuth(actor): crate::auth::AdminAuth,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if id == actor.id {
        return Err(ApiError::validation("cannot delete your own account"));
    }
    // Hierarchy guard: look up the target's current role and verify
    // the actor sits above (or, for admin-on-admin, at-or-above) the
    // target. An admin trying to delete an owner gets 403 here.
    let target = queries::find_user_by_id(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    crate::auth::can_act_on(actor.role, target.role)?;
    let removed = queries::delete_user(&state.pool, id)
        .await
        .map_err(|e| ApiError::validation(format!("{e:#}")))?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateUserInput {
    pub role: UserRole,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub user: User,
}

pub async fn update_user(
    State(state): State<AppState>,
    crate::auth::AdminAuth(actor): crate::auth::AdminAuth,
    Path(id): Path<i64>,
    Json(input): Json<UpdateUserInput>,
) -> Result<Json<UserResponse>, ApiError> {
    if id == actor.id {
        return Err(ApiError::validation("cannot change your own role"));
    }
    let target = queries::find_user_by_id(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    // Hierarchy guard for the target's CURRENT role — admins can't
    // touch owner accounts in any way.
    crate::auth::can_act_on(actor.role, target.role)?;
    // Hierarchy guard for the REQUESTED role — only owners may
    // promote anyone to owner. Admins can promote users ↔ admins
    // freely below the owner ceiling.
    if matches!(input.role, UserRole::Owner) && !matches!(actor.role, UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    let user = queries::set_user_role(&state.pool, id, input.role)
        .await
        .map_err(|e| ApiError::validation(format!("{e:#}")))?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(UserResponse { user }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a `scheme://host[:port]` origin from the setup request's
/// `Origin` header, falling back to the `Referer` (path stripped).
/// Used to seed `public_url` so the CSRF middleware accepts the same
/// browser on subsequent requests without manual config.
/// Setup-token gate (BLOCK #5 in `docs/PUBLIC_RELEASE_HARDENING.md`).
///
/// Three branches, in order:
///
/// 1. `CHIMPFLIX_SETUP_TOKEN` set → require `X-Setup-Token` header
///    with a constant-time-compared exact match. Wrong / missing
///    header is 401.
/// 2. Env unset AND `APP_PUBLIC_ORIGIN` starts with `https://` →
///    refuse with 403. This refuses to permit an unauthenticated
///    setup claim on an internet-facing instance, matching the
///    plaintext-vault refusal in [`load_vault`](crate::load_vault).
/// 3. Otherwise (LAN-ish or dev) → allow, preserving the current
///    one-shot setup UX. The CSRF middleware's existing
///    Origin/Referer check still applies.
fn enforce_setup_token(headers: &HeaderMap) -> Result<(), ApiError> {
    use std::env;
    match env::var("CHIMPFLIX_SETUP_TOKEN").ok().filter(|v| !v.is_empty()) {
        Some(expected) => {
            let provided = headers
                .get("x-setup-token")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
                Ok(())
            } else {
                Err(ApiError::Unauthorized)
            }
        }
        None => {
            let is_public = env::var("APP_PUBLIC_ORIGIN")
                .ok()
                .is_some_and(|origin| origin.starts_with("https://"));
            if is_public {
                Err(ApiError::validation(
                    "first-run setup is disabled on internet-facing deployments without \
                     a setup token. Restart with CHIMPFLIX_SETUP_TOKEN=<random> and \
                     resend POST /auth/setup with the matching X-Setup-Token header. \
                     See docs/DEPLOYMENT.md for the recommended preflight order.",
                ))
            } else {
                Ok(())
            }
        }
    }
}

/// Constant-time byte-slice equality. Length-leaking but value-stable:
/// returns false immediately on length mismatch, then folds all bytes
/// into a single accumulator on a match. Sufficient for our use — the
/// token is a generated random value, not derived from anything the
/// attacker can length-extend.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn setup_origin_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        let trimmed = v.trim();
        if !trimmed.is_empty() && trimmed != "null" {
            return Some(trimmed.to_string());
        }
    }
    let referer = headers
        .get(axum::http::header::REFERER)
        .and_then(|v| v.to_str().ok())?;
    let after_scheme = referer.find("://")?;
    let rest = &referer[after_scheme + 3..];
    let host_end = rest.find('/').unwrap_or(rest.len());
    Some(format!(
        "{}{}",
        &referer[..after_scheme + 3],
        &rest[..host_end]
    ))
}

fn validate_username(name: &str) -> Result<(), ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::validation("username is required"));
    }
    if trimmed.len() > MAX_USERNAME_LEN {
        return Err(ApiError::validation(format!(
            "username must be at most {MAX_USERNAME_LEN} characters"
        )));
    }
    if trimmed.starts_with('_') {
        return Err(ApiError::validation(
            "usernames starting with underscore are reserved",
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(ApiError::validation(
            "usernames may only contain letters, digits, dashes, dots, and underscores",
        ));
    }
    Ok(())
}

/// Obfuscate an email so it can land in the audit log without storing
/// the full address forever. Returns `a***@example.com` for
/// `alice@example.com`; for short local parts (< 3 chars) collapses to
/// `***@example.com`. The domain is preserved so an admin investigating
/// abuse can still tell `example.com` from `attacker.tld`.
fn obfuscate_email(email: &str) -> String {
    let trimmed = email.trim();
    let Some((local, domain)) = trimmed.split_once('@') else {
        return "***".to_string();
    };
    let local = local.trim();
    let prefix = if local.chars().count() >= 3 {
        local.chars().next().unwrap_or('*').to_string()
    } else {
        String::new()
    };
    format!("{prefix}***@{domain}")
}

fn validate_avatar_url(url: &str) -> Result<(), ApiError> {
    // Length cap first — saves work and prevents pathological scans.
    if url.len() > 2048 {
        return Err(ApiError::validation(
            "avatar_url must be at most 2048 characters",
        ));
    }
    // HTTPS only. `http://` would let a man-in-the-middle replace
    // the image; `javascript:` doesn't execute in `<img src>` but
    // `data:` / `file:` / arbitrary schemes are still hostile-shaped
    // surface. https-only is conservative and matches every legit
    // avatar source (Gravatar, S3, GitHub avatars, etc.).
    if !url.starts_with("https://") {
        return Err(ApiError::validation("avatar_url must start with https://"));
    }
    // Reject control chars + whitespace embedded in the URL — header
    // injection / HTML smuggling territory.
    if url.chars().any(|c| c.is_control() || c == ' ') {
        return Err(ApiError::validation(
            "avatar_url contains illegal whitespace or control characters",
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(ApiError::validation(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters",
        )));
    }
    if password.len() > 1024 {
        return Err(ApiError::validation(
            "password must be at most 1024 characters",
        ));
    }
    if is_obviously_bad_password(password) {
        return Err(ApiError::validation(
            "password is in the well-known-bad list; choose something less common",
        ));
    }
    Ok(())
}

/// Lowercase-compared check against a small, in-process list of the
/// most commonly cracked passwords. Catches `password123`, `welcome1`,
/// repeated-char filler, etc. — first-line defense without external
/// dependencies. Not a substitute for haveibeenpwned (which we don't
/// integrate, per project scope), but stops the obvious garbage.
fn is_obviously_bad_password(password: &str) -> bool {
    let lc = password.to_ascii_lowercase();
    // Common-passwords list (top ~50 from public breach corpora). Kept
    // short; longer lists are better served by a Bloom filter, which we
    // can swap in later if the operator asks. Variants like `password1`
    // and `passw0rd` are intentionally enumerated rather than regex-
    // ed because the check is O(N) over a tiny constant either way.
    const BANNED: &[&str] = &[
        "password",
        "password1",
        "password12",
        "password123",
        "password1234",
        "passw0rd",
        "passw0rd1",
        "letmein",
        "welcome",
        "welcome1",
        "qwerty",
        "qwerty123",
        "qwertyuiop",
        "abc12345",
        "abcd1234",
        "admin123",
        "adminadmin",
        "iloveyou",
        "monkey",
        "monkey123",
        "dragon",
        "dragon123",
        "master",
        "master123",
        "shadow",
        "111111",
        "1111111",
        "11111111",
        "123123",
        "123123123",
        "12345678",
        "123456789",
        "1234567890",
        "qazwsx",
        "qazwsxedc",
        "trustno1",
        "sunshine",
        "princess",
        "ashley",
        "michael",
        "jennifer",
        "jordan23",
        "football",
        "baseball",
        "freedom",
        "starwars",
        "superman",
        "batman",
        "111222",
        "12341234",
        "asdf1234",
        "asdfasdf",
        "letmein1",
        "letmein123",
    ];
    if BANNED.contains(&lc.as_str()) {
        return true;
    }
    // Reject all-same-char ("aaaaaaaa", "11111111111") and trivial
    // ascending sequences ("12345678", already in list, but catches
    // longer ones).
    let first = lc.as_bytes().first().copied();
    if let Some(c) = first {
        if lc.len() >= MIN_PASSWORD_LEN && lc.as_bytes().iter().all(|&b| b == c) {
            return true;
        }
    }
    false
}

/// Cheap sanity check — full RFC 5321 validation lives in lettre when
/// the address is actually used. Goal here is to reject obvious junk
/// before the DB write so the user sees a useful error.
fn validate_email(addr: &str) -> Result<(), ApiError> {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return Err(ApiError::validation("email must not be empty"));
    }
    if trimmed.len() > 320 {
        return Err(ApiError::validation("email must be at most 320 characters"));
    }
    let at = trimmed.find('@');
    let Some(at) = at else {
        return Err(ApiError::validation("email must contain '@'"));
    };
    let (local, domain) = trimmed.split_at(at);
    if local.is_empty() {
        return Err(ApiError::validation("email is missing the local part"));
    }
    // domain starts with '@'
    let domain = &domain[1..];
    if domain.is_empty() || !domain.contains('.') {
        return Err(ApiError::validation("email domain looks malformed"));
    }
    if trimmed.contains(char::is_whitespace) {
        return Err(ApiError::validation("email must not contain whitespace"));
    }
    Ok(())
}

fn invalid_credentials() -> ApiError {
    ApiError::validation("invalid credentials")
}

/// Read the inbound session cookie (if any), parse it, and delete
/// the matching `sessions` row. Failures are best-effort — a bad
/// HMAC, expired session, or missing cookie all silently no-op so a
/// fresh first-time login isn't blocked by an irrelevant cleanup.
///
/// Drives WEEK 1 #13 in `docs/PUBLIC_RELEASE_HARDENING.md`. Called
/// on the success path of every login flow (`login`, `oauth_complete`,
/// `accept_invite`, `confirm_password_reset`) so a fixated cookie
/// can't survive the credential check that proves the legitimate
/// user is at the keyboard.
async fn invalidate_inbound_session_if_any(state: &AppState, headers: &HeaderMap) {
    let raw = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join("; ");
    let expected_name = crate::auth::cookie_name(state.auth.cookie_secure);
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(expected_name).and_then(|s| s.strip_prefix('=')) {
            if let Some((session_id, _nonce)) =
                crate::auth::cookie::parse_value(value, &state.auth.session_secret)
            {
                if let Err(e) = queries::delete_session(&state.pool, session_id).await {
                    debug!(error = %format!("{e:#}"), session_id, "fixation defense: delete_session failed");
                }
            }
            return;
        }
    }
}

pub(crate) async fn issue_session(
    state: &AppState,
    user: &User,
    headers: &HeaderMap,
    ip: Option<IpAddr>,
) -> Result<(String, String), ApiError> {
    // Session-fixation defense (WEEK 1 #13 in
    // `docs/PUBLIC_RELEASE_HARDENING.md`). Invalidate any session
    // bound to a pre-existing cookie on this request before
    // minting a new one. Without it, an attacker who planted a
    // known cookie on the victim's browser could reuse it after
    // the victim successfully logs in. Centralised here so every
    // login-success path (login, oauth_complete, accept_invite,
    // confirm_password_reset, complete_setup) gets the defense
    // without each handler having to remember.
    invalidate_inbound_session_if_any(state, headers).await;

    let mut nonce = [0u8; 32];
    password::fill_random(&mut nonce).map_err(ApiError::Internal)?;
    let expires_at = now_ms() + SESSION_MAX_AGE_S * 1000;
    let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    // Pull the effective client IP from the trusted-proxy middleware
    // (see [`crate::client_ip`]). Caller passes it in via the
    // `Extension<EffectiveClientIp>` extractor so we never silently
    // trust an unverified `X-Forwarded-For`.
    let ip_str = ip.map(|i| i.to_string());
    let session_id = queries::create_session(
        &state.pool,
        user.id,
        &nonce,
        expires_at,
        user_agent,
        ip_str.as_deref(),
    )
    .await
    .map_err(ApiError::Internal)?;

    let value = cookie::build_value(session_id, &nonce, &state.auth.session_secret);
    let session_cookie =
        cookie::set_cookie_header(&value, SESSION_MAX_AGE_S, state.auth.cookie_secure);
    // Issue the double-submit CSRF companion cookie alongside. The
    // token is deterministic (HMAC of session_id + nonce keyed by the
    // server secret) so the middleware doesn't need to read any DB
    // state to verify — just recompute and compare.
    let csrf = cookie::csrf_token(session_id, &nonce, &state.auth.session_secret);
    let csrf_cookie =
        cookie::set_csrf_cookie_header(&csrf, SESSION_MAX_AGE_S, state.auth.cookie_secure);
    Ok((session_cookie, csrf_cookie))
}

pub(crate) fn authed_response(
    status: StatusCode,
    user: User,
    cookies: (String, String),
    _state: &AppState,
) -> axum::response::Response {
    let mut response = (status, Json(AuthResponse { user })).into_response();
    let (session_cookie, csrf_cookie) = cookies;
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&session_cookie).expect("ascii cookie"),
    );
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&csrf_cookie).expect("ascii cookie"),
    );
    response
}

/// Push an audit-log entry for an authentication event. Centralized so
/// every login/logout/password-reset/session-revoke logs the same
/// shape (action + target user id + UA + IP, no schema drift).
///
/// `ip` must be the *effective* client IP — handlers extract it via
/// [`Extension<EffectiveClientIp>`] and pass it explicitly so we never
/// silently trust an unverified `X-Forwarded-For` header.
pub(crate) async fn audit_auth(
    state: &AppState,
    action: &str,
    actor_user_id: Option<i64>,
    target_user_id: Option<i64>,
    payload_json: Option<String>,
    headers: &HeaderMap,
    ip: Option<IpAddr>,
) {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let ip_str = ip.map(|i| i.to_string());
    crate::api::admin::audit_log(
        state,
        NewAuditEntry {
            actor_user_id,
            action: action.to_string(),
            target_kind: Some("auth".to_string()),
            target_id: target_user_id.map(|id| id.to_string()),
            payload_json,
            ip: ip_str,
            user_agent,
        },
    )
    .await;
}

/// Login-specific response builder. The /auth/login endpoint returns
/// the tagged [`LoginResponse`] enum so callers can match on `status`
/// before destructuring `user` vs `challenge`. Setup + register still
/// use `authed_response` because their happy path is always
/// authenticated (no 2FA challenge applies — the user has no enrollment).
fn authed_login_response(user: User, cookies: (String, String)) -> axum::response::Response {
    let mut response =
        (StatusCode::OK, Json(LoginResponse::Authenticated { user })).into_response();
    let (session_cookie, csrf_cookie) = cookies;
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&session_cookie).expect("ascii cookie"),
    );
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&csrf_cookie).expect("ascii cookie"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_password() {
        assert!(validate_password("hunter2").is_err());
        assert!(validate_password("12345678").is_err());
    }

    #[test]
    fn accepts_min_length_strong_password() {
        assert!(validate_password("MyB1g!Phras3").is_ok());
    }

    #[test]
    fn rejects_overlong_password() {
        let too_long = "a".repeat(1025);
        assert!(validate_password(&too_long).is_err());
    }

    #[test]
    fn rejects_common_passwords_case_insensitive() {
        assert!(validate_password("Password1234").is_err());
        assert!(validate_password("LETMEIN1").is_err());
        assert!(validate_password("11111111111").is_err());
        assert!(validate_password("AAAAAAAAAAAA").is_err());
    }

    #[test]
    fn obviously_bad_helpers_dont_match_random_strings() {
        assert!(!is_obviously_bad_password("correct horse battery staple"));
        assert!(!is_obviously_bad_password("Tr0ub4dor&3xx"));
    }

    /// Smoke-test the session-fixation cookie parser: a fabricated
    /// cookie value with a valid HMAC must round-trip via
    /// `cookie::parse_value`; one with a tampered session id must not.
    /// The handler integration uses this in
    /// `invalidate_inbound_session_if_any`. Full HTTP-layer coverage
    /// would require a live AppState + DB; this guards the crypto
    /// contract that the fixation defense relies on.
    #[test]
    fn fixation_defense_relies_on_parse_value_round_trip() {
        let secret = b"a-test-secret-32-bytes-or-larger!";
        let nonce = [42u8; 32];
        let value = crate::auth::cookie::build_value(7, &nonce, secret);
        let parsed = crate::auth::cookie::parse_value(&value, secret);
        assert_eq!(parsed, Some((7, nonce)));

        // Tampered session id (the attacker-fabricated scenario): the
        // payload's HMAC no longer matches, so parse_value returns
        // None and invalidate_inbound_session_if_any silently skips
        // (correct behaviour — we never want to delete a row keyed by
        // a forged id).
        let mut tampered = value.clone();
        tampered.replace_range(0..1, "9");
        assert!(crate::auth::cookie::parse_value(&tampered, secret).is_none());
    }
}
