//! `/api/v1/auth/2fa/*` handlers — TOTP enrollment, verification,
//! disable, and the second-step login challenge.
//!
//! Flow:
//!   1. User opens Settings → Two-Factor → "Set up"
//!   2. POST /auth/2fa/enroll {password}     → server returns otpauth URI
//!   3. User scans QR in their authenticator
//!   4. POST /auth/2fa/verify {code}         → server marks verified +
//!      returns 10 recovery codes (shown once)
//!   5. From now on, every login goes:
//!        POST /auth/login                   → {status: "2fa_required", challenge}
//!        POST /auth/2fa/login {challenge,code|recovery_code} → session
//!
//! Enroll/disable/regenerate-codes all require the password re-entry —
//! a stolen session shouldn't be enough to weaken the account's 2FA.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::{SET_COOKIE, USER_AGENT};
use axum::http::{HeaderValue, StatusCode};
use axum::response::IntoResponse;
use chimpflix_common::now_ms;
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::api::error::ApiError;
use crate::auth::{AuthUser, SESSION_MAX_AGE_S, cookie, password};
use crate::state::AppState;
use crate::totp;

const CHALLENGE_RECOVERY_KEY: &str = "recovery";

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    /// True when a TOTP row exists for the user, regardless of verification.
    pub enrolled: bool,
    /// True when the user has completed the enroll → verify handshake.
    /// Only verified users are challenged at login.
    pub verified: bool,
    /// Global policy. Mirrors `server_settings.totp_enforcement` so the
    /// UI can render "required" / "optional" / "disabled" hints next to
    /// the enroll button.
    pub enforcement: String,
    /// Count of unused recovery codes remaining. Surfaced so the UI can
    /// nudge regeneration when low.
    pub unused_recovery_codes: i64,
}

pub async fn status(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<StatusResponse>, ApiError> {
    let record = queries::get_user_totp(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    let unused = queries::count_unused_recovery_codes(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    let enforcement = state.settings.read().await.totp_enforcement.clone();
    Ok(Json(StatusResponse {
        enrolled: record.is_some(),
        verified: record.as_ref().is_some_and(|r| r.verified_at.is_some()),
        enforcement,
        unused_recovery_codes: unused,
    }))
}

#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    /// Re-entered current password. We require this even though the
    /// user is already authenticated so a stolen session can't bind a
    /// new TOTP secret to the account.
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    /// Base32 of the shared secret — surfaced for manual entry when the
    /// user's authenticator can't scan the QR.
    pub secret: String,
    /// `otpauth://totp/...` URI the client renders as a QR.
    pub otpauth_uri: String,
    /// Pre-rendered QR as a `data:image/svg+xml;base64,…` URL the
    /// frontend drops straight into <img src>. Lets us avoid a
    /// client-side QR encoder dep.
    pub qr_data_url: String,
}

pub async fn enroll(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Json(input): Json<EnrollRequest>,
) -> Result<Json<EnrollResponse>, ApiError> {
    if state.settings.read().await.totp_enforcement == "disabled" {
        return Err(ApiError::validation("2FA enrollment is disabled by the server administrator"));
    }
    reverify_password(&state, user.id, &input.password).await?;

    let issuer = state.settings.read().await.server_name.clone();
    let account = user.username.clone();
    let material = totp::generate_enrollment(&state.vault, &issuer, &account)
        .map_err(ApiError::Internal)?;
    queries::upsert_user_totp(
        &state.pool,
        user.id,
        &material.blob.value,
        material.blob.nonce.as_deref(),
    )
    .await
    .map_err(ApiError::Internal)?;

    info!(user_id = user.id, "2fa enrollment started");
    crate::api::auth::audit_auth(
        &state,
        "auth.2fa.enroll",
        Some(user.id),
        Some(user.id),
        None,
        &headers,
    )
    .await;
    Ok(Json(EnrollResponse {
        secret: material.secret_b32,
        otpauth_uri: material.otpauth_uri,
        qr_data_url: material.qr_data_url,
    }))
}

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    /// 6-digit TOTP code from the authenticator app.
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    /// Recovery codes — shown ONCE. The server only stores SHA-256 hashes.
    pub recovery_codes: Vec<String>,
}

pub async fn verify(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Json(input): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, ApiError> {
    let record = queries::get_user_totp(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::validation("no enrollment in progress — call /enroll first"))?;
    let secret = totp::decrypt_secret(
        &state.vault,
        &record.secret_enc,
        record.secret_nonce.as_deref(),
    )
    .map_err(ApiError::Internal)?;
    if !totp::verify_code(&secret, &input.code).map_err(ApiError::Internal)? {
        return Err(ApiError::validation("invalid code"));
    }

    // Mark verified + replace recovery codes in one logical step.
    queries::mark_user_totp_verified(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    let codes = totp::generate_recovery_codes(totp::RECOVERY_CODE_COUNT);
    let hashes: Vec<String> = codes.iter().map(|(_, h)| h.clone()).collect();
    queries::replace_recovery_codes(&state.pool, user.id, &hashes)
        .await
        .map_err(ApiError::Internal)?;

    // Session rotation: enrolling 2FA increases account security, so any
    // other live session that established before enrollment shouldn't
    // benefit retroactively. Keep the current session alive; punt the rest.
    if let Err(e) =
        queries::delete_sessions_for_user_except(&state.pool, user.id, user.session_id).await
    {
        warn!(error = %format!("{e:#}"), user_id = user.id, "session rotation on 2FA verify");
    }

    info!(user_id = user.id, "2fa enrolled + recovery codes issued");
    crate::api::auth::audit_auth(
        &state,
        "auth.2fa.verify",
        Some(user.id),
        Some(user.id),
        None,
        &headers,
    )
    .await;
    Ok(Json(VerifyResponse {
        recovery_codes: codes.into_iter().map(|(p, _)| p).collect(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct DisableRequest {
    pub password: String,
}

pub async fn disable(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Json(input): Json<DisableRequest>,
) -> Result<StatusCode, ApiError> {
    if state.settings.read().await.totp_enforcement == "required" {
        return Err(ApiError::validation(
            "2FA is required by server policy and cannot be disabled",
        ));
    }
    reverify_password(&state, user.id, &input.password).await?;
    let removed = queries::delete_user_totp(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    info!(user_id = user.id, "2fa disabled");

    // Disabling 2FA is a security downgrade — invalidate other sessions
    // so a compromised side-channel can't ride the change.
    if let Err(e) =
        queries::delete_sessions_for_user_except(&state.pool, user.id, user.session_id).await
    {
        warn!(error = %format!("{e:#}"), user_id = user.id, "session rotation on 2FA disable");
    }

    crate::api::auth::audit_auth(
        &state,
        "auth.2fa.disable",
        Some(user.id),
        Some(user.id),
        None,
        &headers,
    )
    .await;
    // Notify admins so they notice if a user voluntarily downgrades.
    // Fetch the full user record after the delete so display_name etc.
    // are current.
    if let Ok(Some(full)) = queries::find_user_by_id(&state.pool, user.id).await {
        crate::notifier::notify_two_factor_disabled(&state, &full).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct RegenerateRecoveryRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct RegenerateRecoveryResponse {
    pub recovery_codes: Vec<String>,
}

pub async fn regenerate_recovery_codes(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Json(input): Json<RegenerateRecoveryRequest>,
) -> Result<Json<RegenerateRecoveryResponse>, ApiError> {
    reverify_password(&state, user.id, &input.password).await?;

    // Only meaningful if the user is enrolled + verified — otherwise
    // recovery codes have nothing to recover.
    let record = queries::get_user_totp(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::validation("2FA is not enabled"))?;
    if record.verified_at.is_none() {
        return Err(ApiError::validation("2FA enrollment isn't verified yet"));
    }

    let codes = totp::generate_recovery_codes(totp::RECOVERY_CODE_COUNT);
    let hashes: Vec<String> = codes.iter().map(|(_, h)| h.clone()).collect();
    queries::replace_recovery_codes(&state.pool, user.id, &hashes)
        .await
        .map_err(ApiError::Internal)?;
    crate::api::auth::audit_auth(
        &state,
        "auth.2fa.recovery_regenerate",
        Some(user.id),
        Some(user.id),
        None,
        &headers,
    )
    .await;
    Ok(Json(RegenerateRecoveryResponse {
        recovery_codes: codes.into_iter().map(|(p, _)| p).collect(),
    }))
}

// ---------------------------------------------------------------------------
// Step 2 of login — redeem a TOTP challenge for a real session cookie.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ChallengeLoginRequest {
    pub challenge: String,
    /// Exactly one of `code` or `recovery_code` must be present. We
    /// don't enforce that at the type level — handler returns a generic
    /// "invalid code" if both are missing or empty.
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub recovery_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    user: chimpflix_library::User,
}

pub async fn challenge_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ChallengeLoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user_id = totp::parse_challenge(
        &input.challenge,
        &state.auth.session_secret,
        now_ms(),
    )
    .ok_or_else(invalid_credentials)?;

    let attempt_key = format!("2fa:{user_id}");
    if let Some(wait) = state.login_attempts.check(&attempt_key).await {
        return Err(ApiError::TooManyRequests(format!(
            "too many failed 2FA attempts; try again in {}s",
            wait.as_secs().max(1)
        )));
    }

    let user = queries::find_user_by_id(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(invalid_credentials)?;

    // Pull the enrollment record. If 2FA was disabled between step-1
    // and step-2, the challenge is stale — refuse.
    let record = queries::get_user_totp(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(invalid_credentials)?;
    if record.verified_at.is_none() {
        return Err(invalid_credentials());
    }

    let ok = match (input.code.as_deref(), input.recovery_code.as_deref()) {
        (Some(code), _) if !code.trim().is_empty() => {
            let secret = totp::decrypt_secret(
                &state.vault,
                &record.secret_enc,
                record.secret_nonce.as_deref(),
            )
            .map_err(ApiError::Internal)?;
            totp::verify_code(&secret, code).map_err(ApiError::Internal)?
        }
        (_, Some(rc)) if !rc.trim().is_empty() => {
            let hash = totp::hash_recovery_code(rc);
            queries::consume_recovery_code(&state.pool, user_id, &hash)
                .await
                .map_err(ApiError::Internal)?
        }
        _ => false,
    };

    if !ok {
        state.login_attempts.record_failure(&attempt_key).await;
        crate::api::auth::audit_auth(
            &state,
            "auth.2fa.login.failure",
            None,
            Some(user_id),
            None,
            &headers,
        )
        .await;
        return Err(invalid_credentials());
    }
    state.login_attempts.record_success(&attempt_key).await;

    let ip = crate::api::rate_limit::header_client_ip(&headers);
    if let Err(e) = queries::record_user_login(&state.pool, user.id, ip.as_deref()).await {
        warn!(error = %format!("{e:#}"), user_id = user.id, "record_user_login");
    }
    let cookie_value = issue_session(&state, &user, &headers).await?;
    let used_recovery = input
        .recovery_code
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    info!(user_id, %used_recovery, "2fa login");
    crate::api::auth::audit_auth(
        &state,
        "auth.2fa.login.success",
        Some(user_id),
        Some(user_id),
        Some(format!(r#"{{"used_recovery":{used_recovery}}}"#)),
        &headers,
    )
    .await;

    let mut response = (StatusCode::OK, Json(AuthResponse { user })).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie_value).expect("ascii cookie"),
    );
    // Telemetry-light header — lets the client surface "we used your
    // recovery code, regenerate them" without bloating the body.
    if used_recovery {
        if let Ok(v) = HeaderValue::from_str("1") {
            response
                .headers_mut()
                .insert(axum::http::HeaderName::from_static("x-chimpflix-recovery-used"), v);
        }
    }
    // Suppress unused-constant warning until something needs to look at
    // the recovery-vs-code distinction beyond logging.
    let _ = CHALLENGE_RECOVERY_KEY;
    Ok(response)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn reverify_password(
    state: &AppState,
    user_id: i64,
    password: &str,
) -> Result<(), ApiError> {
    if password.is_empty() {
        return Err(ApiError::validation("password is required"));
    }
    let user = queries::find_user_by_id(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::Unauthorized)?;
    let record = queries::find_user_with_secret_by_username(&state.pool, &user.username)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::Unauthorized)?;
    if !password::verify(password, &record.password_hash) {
        warn!(user_id, "password re-verification failed");
        return Err(ApiError::Unauthorized);
    }
    Ok(())
}

async fn issue_session(
    state: &AppState,
    user: &chimpflix_library::User,
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

fn invalid_credentials() -> ApiError {
    ApiError::validation("invalid credentials")
}
