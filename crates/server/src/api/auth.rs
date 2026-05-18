//! /api/v1/auth handlers: setup, login, logout, me, register, invites.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::{SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use chimpflix_common::now_ms;
use chimpflix_library::queries;
use chimpflix_library::{
    CreateInviteInput, Invite, LoginInput, NewAuditEntry, RegisterInput, SessionSummary,
    SetupInput, User, UserRole, hash_invite_code,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tracing::{info, warn};

use crate::api::error::ApiError;
use crate::auth::{AuthUser, OwnerAuth, SESSION_MAX_AGE_S, cookie, password};
use crate::mailer::{Mailer, OutgoingMessage};
use crate::state::AppState;

const MIN_PASSWORD_LEN: usize = 8;
const MAX_USERNAME_LEN: usize = 64;

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    user: User,
}

/// Login outcomes. Tagged by `status` so clients can match on the
/// discriminator before destructuring:
///   * `authenticated` — session cookie set, `user` populated.
///   * `2fa_required` — no cookie, follow up with /auth/2fa/login.
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
    headers: HeaderMap,
    Json(input): Json<SetupInput>,
) -> Result<impl IntoResponse, ApiError> {
    if !queries::is_in_setup_mode(&state.pool)
        .await
        .map_err(ApiError::Internal)?
    {
        return Err(ApiError::Forbidden);
    }
    validate_username(&input.username)?;
    validate_password(&input.password)?;
    let email = input.email.as_deref().map(str::trim).filter(|s| !s.is_empty());
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

    let cookie_value = issue_session(&state, &user, &headers).await?;
    info!(user_id = user.id, "setup complete");
    Ok(authed_response(
        StatusCode::CREATED,
        user,
        cookie_value,
        &state,
    ))
}

// ---------------------------------------------------------------------------
// Login / logout / me
// ---------------------------------------------------------------------------

pub async fn login(
    State(state): State<AppState>,
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
        )
        .await;
        return Err(invalid_credentials());
    }
    let user = user_opt.expect("ok=true requires a user");
    state.login_attempts.record_success(&attempt_key).await;

    // 2FA check — if the user has a verified TOTP enrollment, don't
    // issue a session yet. Return a short-lived signed challenge that
    // the client trades for a session via /auth/2fa/login. The session
    // cookie is only set once the second factor is proven, so a
    // password-only compromise can't establish a session.
    let totp_record = queries::get_user_totp(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    if totp_record.as_ref().is_some_and(|r| r.verified_at.is_some()) {
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
        )
        .await;
        return Ok(Json(LoginResponse::TwoFactorRequired {
            challenge,
            expires_in_seconds: crate::totp::CHALLENGE_TTL_SECS,
        })
        .into_response());
    }

    let ip = crate::api::rate_limit::header_client_ip(&headers);
    if let Err(e) = queries::record_user_login(&state.pool, user.id, ip.as_deref()).await {
        warn!(error = %format!("{e:#}"), user_id = user.id, "record_user_login");
    }
    let cookie_value = issue_session(&state, &user, &headers).await?;
    info!(user_id = user.id, "login");
    audit_auth(
        &state,
        "auth.login.success",
        Some(user.id),
        Some(user.id),
        None,
        &headers,
    )
    .await;
    Ok(authed_login_response(user, cookie_value))
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
            MySessionEntry { session: s, current }
        })
        .collect();
    Ok(Json(MySessionsResponse { sessions }))
}

pub async fn revoke_my_session(
    State(state): State<AppState>,
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
    user: AuthUser,
    headers: HeaderMap,
) -> Result<Json<RevokeOthersResponse>, ApiError> {
    let revoked =
        queries::delete_sessions_for_user_except(&state.pool, user.id, user.session_id)
            .await
            .map_err(ApiError::Internal)?;
    info!(user_id = user.id, revoked, "user signed out of other sessions");
    audit_auth(
        &state,
        "auth.sessions.revoke_others",
        Some(user.id),
        Some(user.id),
        Some(format!(r#"{{"revoked":{revoked}}}"#)),
        &headers,
    )
    .await;
    Ok(Json(RevokeOthersResponse { revoked }))
}

pub async fn logout(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    // Delete the *current* session row, then clear the cookie.
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Some(raw) = cookie::find_cookie(cookie_header, crate::auth::COOKIE_NAME) {
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
    )
    .await;
    let clear = cookie::clear_cookie_header(state.auth.cookie_secure);
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, HeaderValue::from_str(&clear).unwrap());
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
    /// Single-Option: present → set the boolean. Omit to leave as-is.
    pub notify_via_email: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
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
        )
        .await;
        return Err(ApiError::Unauthorized);
    }

    let new_hash = password::hash(&input.new_password).map_err(ApiError::Internal)?;
    queries::update_user_password(&state.pool, user.id, &new_hash)
        .await
        .map_err(ApiError::Internal)?;
    let revoked =
        queries::delete_sessions_for_user_except(&state.pool, user.id, user.session_id)
            .await
            .unwrap_or(0);
    info!(user_id = user.id, sessions_revoked = revoked, "password changed");
    audit_auth(
        &state,
        "auth.password_change.success",
        Some(user.id),
        Some(user.id),
        Some(format!(r#"{{"sessions_revoked":{revoked}}}"#)),
        &headers,
    )
    .await;
    Ok(Json(ChangePasswordResponse {
        sessions_revoked: revoked,
    }))
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
    let patch = queries::UserSelfUpdate {
        display_name: normalize(input.display_name),
        avatar_url: normalize(input.avatar_url),
        email: email_patch,
        default_audio_lang: normalize(input.default_audio_lang),
        default_subtitle_lang: normalize(input.default_subtitle_lang),
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

    let cookie_value = issue_session(&state, &user, &headers).await?;
    info!(user_id = user.id, "register");
    Ok(authed_response(
        StatusCode::CREATED,
        user,
        cookie_value,
        &state,
    ))
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
                let html = invite_email_html(
                    &server_name,
                    accept_url.as_deref(),
                    &code,
                    expires_at,
                );
                let text = invite_email_text(
                    &server_name,
                    accept_url.as_deref(),
                    &code,
                    expires_at,
                );
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
    let link = accept_url.unwrap_or(code);
    let expiry = expires_at
        .map(|ms| format!("\nThis invite expires at epoch ms {ms}.\n"))
        .unwrap_or_default();
    format!(
        "You've been invited to {server_name}.\n\n\
         Open this link to set up your account:\n  {link}\n\n\
         If the link doesn't work, your invite code is:\n  {code}\n{expiry}\n\
         If you didn't expect this, you can ignore the message.\n"
    )
}

fn invite_email_html(
    server_name: &str,
    accept_url: Option<&str>,
    code: &str,
    expires_at: Option<i64>,
) -> String {
    // The recipient is a normal email client — escape user-controlled
    // values to defang HTML injection from the server_name field.
    let server_safe = html_escape(server_name);
    let code_safe = html_escape(code);
    let link_html = match accept_url {
        Some(url) => {
            let url_safe = html_escape(url);
            format!(
                r#"<p><a href="{url_safe}" style="display:inline-block;background:#e50914;color:#fff;padding:10px 20px;border-radius:4px;text-decoration:none;font-weight:600;">Accept invitation</a></p>
                   <p style="font-size:12px;color:#555">Or paste this link: <code>{url_safe}</code></p>"#
            )
        }
        None => String::new(),
    };
    let expiry_html = expires_at
        .map(|ms| format!(r#"<p style="font-size:12px;color:#555">This invitation expires at epoch ms {ms}.</p>"#))
        .unwrap_or_default();
    format!(
        r#"<!doctype html><html><body style="font-family:system-ui,sans-serif;max-width:560px;margin:24px auto;padding:0 16px;color:#111">
            <h2>You're invited to {server_safe}</h2>
            {link_html}
            <p style="font-size:14px;">If the button or link doesn't work, your invite code is:</p>
            <pre style="background:#f5f5f5;padding:12px;border-radius:4px;font-size:13px;">{code_safe}</pre>
            {expiry_html}
            <p style="font-size:12px;color:#888">If you didn't expect this email, you can safely ignore it.</p>
           </body></html>"#
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// ---------------------------------------------------------------------------
// Email change (verification round-trip)
// ---------------------------------------------------------------------------

const EMAIL_CHANGE_TTL_S: i64 = 60 * 60;

#[derive(Debug, Deserialize)]
pub struct RequestEmailChangeRequest {
    pub new_email: String,
    pub password: String,
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
    queries::consume_email_change(&state.pool, token_id, user.id, &new_email)
        .await
        .map_err(|e| {
            let msg = format!("{e:#}");
            if msg.contains("UNIQUE constraint failed") {
                ApiError::Conflict("that email is already in use by another account".into())
            } else {
                ApiError::validation(format!("{e}"))
            }
        })?;
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
    )
    .await;
    Ok(Json(ConfirmEmailChangeResponse { email: new_email }))
}

fn email_change_text(server_name: &str, verify_url: Option<&str>, token: &str) -> String {
    let link = verify_url.unwrap_or(token);
    format!(
        "Someone (hopefully you) asked to change the {server_name} account email to this address.\n\n\
         Confirm by opening this link:\n  {link}\n\n\
         If the link doesn't work, your confirmation token is:\n  {token}\n\n\
         The link expires in 1 hour. If you didn't request this, you can ignore the email —\n\
         no change has been made.\n"
    )
}

fn email_change_html(server_name: &str, verify_url: Option<&str>, token: &str) -> String {
    let server_safe = html_escape(server_name);
    let token_safe = html_escape(token);
    let link_html = match verify_url {
        Some(url) => {
            let url_safe = html_escape(url);
            format!(
                r#"<p><a href="{url_safe}" style="display:inline-block;background:#e50914;color:#fff;padding:10px 20px;border-radius:4px;text-decoration:none;font-weight:600;">Confirm email</a></p>
                   <p style="font-size:12px;color:#555">Or paste this link: <code>{url_safe}</code></p>"#
            )
        }
        None => String::new(),
    };
    format!(
        r#"<!doctype html><html><body style="font-family:system-ui,sans-serif;max-width:560px;margin:24px auto;padding:0 16px;color:#111">
            <h2>Confirm your new {server_safe} email</h2>
            <p>Someone (hopefully you) asked to change the account email to this address.</p>
            {link_html}
            <p style="font-size:14px;">If the button or link doesn't work, your confirmation token is:</p>
            <pre style="background:#f5f5f5;padding:12px;border-radius:4px;font-size:13px;">{token_safe}</pre>
            <p style="font-size:12px;color:#555">The link expires in 1 hour.</p>
            <p style="font-size:12px;color:#888">If you didn't request this, you can ignore the email — no change has been made.</p>
           </body></html>"#
    )
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
    headers: HeaderMap,
    Json(input): Json<PasswordResetRequest>,
) -> Result<StatusCode, ApiError> {
    let email = input.email.trim();
    // Silently no-op for malformed input — same response shape as the
    // happy path so a probe sees no difference.
    if email.is_empty() || !email.contains('@') || email.len() > 320 {
        return Ok(StatusCode::NO_CONTENT);
    }

    let user_opt = queries::find_user_by_email(&state.pool, email)
        .await
        .map_err(ApiError::Internal)?;

    // Audit the REQUEST whether or not a user matched — same shape
    // either way so an attacker probing the audit log (if they could
    // see it) can't tell which addresses are real.
    audit_auth(
        &state,
        "auth.password_reset.request",
        None,
        user_opt.as_ref().map(|u| u.id),
        Some(format!(
            r#"{{"email":{}}}"#,
            serde_json::Value::String(email.to_string())
        )),
        &headers,
    )
    .await;

    if let Some(user) = user_opt {
        // Generate token + hash, persist hash only.
        let mut buf = [0u8; 32];
        password::fill_random(&mut buf).map_err(ApiError::Internal)?;
        let token = hex::encode(buf);
        let token_hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
        let expires_at = now_ms() + PASSWORD_RESET_TTL_S * 1000;
        let ip = headers
            .get(axum::http::HeaderName::from_static("x-forwarded-for"))
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .map(str::trim);
        let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
        if let Err(e) = queries::create_password_reset_token(
            &state.pool,
            user.id,
            &token_hash,
            expires_at,
            ip,
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
                let html = password_reset_email_html(
                    &server_name,
                    reset_url.as_deref(),
                    &token,
                );
                let text = password_reset_email_text(
                    &server_name,
                    reset_url.as_deref(),
                    &token,
                );
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

#[derive(Debug, Deserialize)]
pub struct PasswordResetConfirm {
    pub token: String,
    pub new_password: String,
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
    headers: HeaderMap,
    Json(input): Json<PasswordResetConfirm>,
) -> Result<Json<PasswordResetConfirmResponse>, ApiError> {
    let token = input.token.trim();
    if token.is_empty() || token.len() > 128 {
        return Err(ApiError::validation("token is invalid or expired"));
    }
    validate_password(&input.new_password)?;

    let token_hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
    let (token_id, user_id) =
        queries::find_active_password_reset_token(&state.pool, &token_hash)
            .await
            .map_err(ApiError::Internal)?
            .ok_or_else(|| ApiError::validation("token is invalid or expired"))?;

    let hash = password::hash(&input.new_password).map_err(ApiError::Internal)?;
    let revoked = queries::consume_password_reset(&state.pool, token_id, user_id, &hash)
        .await
        .map_err(|e| ApiError::validation(format!("{e}")))?;

    info!(user_id, sessions_revoked = revoked, "password reset");
    audit_auth(
        &state,
        "auth.password_reset.confirm",
        Some(user_id),
        Some(user_id),
        Some(format!(r#"{{"sessions_revoked":{revoked}}}"#)),
        &headers,
    )
    .await;
    Ok(Json(PasswordResetConfirmResponse {
        sessions_revoked: revoked,
    }))
}

fn password_reset_email_text(server_name: &str, reset_url: Option<&str>, token: &str) -> String {
    let link = reset_url.unwrap_or(token);
    format!(
        "Someone (hopefully you) requested a password reset for your {server_name} account.\n\n\
         Open this link to set a new password:\n  {link}\n\n\
         If the link doesn't work, your reset token is:\n  {token}\n\n\
         The link expires in 1 hour. If you didn't request this, you can ignore the email — \
         your password hasn't changed.\n"
    )
}

fn password_reset_email_html(server_name: &str, reset_url: Option<&str>, token: &str) -> String {
    let server_safe = html_escape(server_name);
    let token_safe = html_escape(token);
    let link_html = match reset_url {
        Some(url) => {
            let url_safe = html_escape(url);
            format!(
                r#"<p><a href="{url_safe}" style="display:inline-block;background:#e50914;color:#fff;padding:10px 20px;border-radius:4px;text-decoration:none;font-weight:600;">Reset password</a></p>
                   <p style="font-size:12px;color:#555">Or paste this link: <code>{url_safe}</code></p>"#
            )
        }
        None => String::new(),
    };
    format!(
        r#"<!doctype html><html><body style="font-family:system-ui,sans-serif;max-width:560px;margin:24px auto;padding:0 16px;color:#111">
            <h2>Reset your {server_safe} password</h2>
            <p>Someone (hopefully you) requested a password reset.</p>
            {link_html}
            <p style="font-size:14px;">If the button or link doesn't work, your reset token is:</p>
            <pre style="background:#f5f5f5;padding:12px;border-radius:4px;font-size:13px;">{token_safe}</pre>
            <p style="font-size:12px;color:#555">The link expires in 1 hour.</p>
            <p style="font-size:12px;color:#888">If you didn't request this, you can ignore the email — your password hasn't changed.</p>
           </body></html>"#
    )
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
    OwnerAuth(owner): OwnerAuth,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if id == owner.id {
        return Err(ApiError::validation("cannot delete the owner account"));
    }
    let removed = queries::delete_user(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
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
    OwnerAuth(owner): OwnerAuth,
    Path(id): Path<i64>,
    Json(input): Json<UpdateUserInput>,
) -> Result<Json<UserResponse>, ApiError> {
    if id == owner.id {
        return Err(ApiError::validation("cannot change your own role"));
    }
    let user = queries::set_user_role(&state.pool, id, input.role)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(UserResponse { user }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
    Ok(())
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

async fn issue_session(
    state: &AppState,
    user: &User,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let mut nonce = [0u8; 32];
    password::fill_random(&mut nonce).map_err(ApiError::Internal)?;
    let expires_at = now_ms() + SESSION_MAX_AGE_S * 1000;
    let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    let session_id =
        queries::create_session(&state.pool, user.id, &nonce, expires_at, user_agent, None)
            .await
            .map_err(ApiError::Internal)?;

    let value = cookie::build_value(session_id, &nonce, &state.auth.session_secret);
    Ok(cookie::set_cookie_header(
        &value,
        SESSION_MAX_AGE_S,
        state.auth.cookie_secure,
    ))
}

fn authed_response(
    status: StatusCode,
    user: User,
    cookie_header: String,
    _state: &AppState,
) -> axum::response::Response {
    let mut response = (status, Json(AuthResponse { user })).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie_header).expect("ascii cookie"),
    );
    response
}

/// Push an audit-log entry for an authentication event. Centralized so
/// every login/logout/password-reset/session-revoke logs the same
/// shape (action + target user id + UA + IP, no schema drift).
pub(crate) async fn audit_auth(
    state: &AppState,
    action: &str,
    actor_user_id: Option<i64>,
    target_user_id: Option<i64>,
    payload_json: Option<String>,
    headers: &HeaderMap,
) {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let ip = crate::api::rate_limit::header_client_ip(headers);
    crate::api::admin::audit_log(
        state,
        NewAuditEntry {
            actor_user_id,
            action: action.to_string(),
            target_kind: Some("auth".to_string()),
            target_id: target_user_id.map(|id| id.to_string()),
            payload_json,
            ip,
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
fn authed_login_response(user: User, cookie_header: String) -> axum::response::Response {
    let mut response = (
        StatusCode::OK,
        Json(LoginResponse::Authenticated { user }),
    )
        .into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie_header).expect("ascii cookie"),
    );
    response
}
