//! /admin/users*, /admin/sessions*, /admin/access-matrix — Phase 8 surface.
//!
//! User CRUD + invites live under `/admin/users*` and `/admin/invites*`;
//! handlers are defined in the `auth` module but mounted under `/admin`
//! since both endpoints are admin-only.

use axum::Extension;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, StatusCode};
use chimpflix_library::{AccessLevel, AccessMatrixEntry, NewAuditEntry, SessionSummary, queries};

use crate::client_ip::EffectiveClientIp;
use serde::{Deserialize, Serialize};

use sha2::Digest as _;

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::{AdminAuth, AuthUser, OwnerAuth, can_act_on};
use crate::mail_template;
use crate::mailer::{Mailer, OutgoingMessage};
use crate::state::AppState;

/// Resolve the target user's current role and reject the request if
/// the actor isn't allowed to manage them. Returns the loaded `User`
/// on success so callers can use display_name / email without a
/// second round trip.
async fn require_target(
    state: &AppState,
    actor_role: chimpflix_library::UserRole,
    target_id: i64,
) -> Result<chimpflix_library::User, ApiError> {
    let target = queries::find_user_by_id(&state.pool, target_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    can_act_on(actor_role, target.role)?;
    Ok(target)
}

#[derive(Debug, Serialize)]
pub struct SessionsListResponse {
    pub sessions: Vec<SessionSummary>,
}

pub async fn list_sessions(
    State(state): State<AppState>,
    AdminAuth(actor): AdminAuth,
) -> Result<Json<SessionsListResponse>, ApiError> {
    // Non-Owner Admins shouldn't be able to enumerate the Owner's
    // active sessions (their IPs, UAs, and the bare existence of
    // current logins). Owners see everything; Admins see Admins + Users.
    let exclude_owners = !matches!(actor.role, chimpflix_library::UserRole::Owner);
    let sessions = queries::list_all_sessions_filtered(&state.pool, exclude_owners)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(SessionsListResponse { sessions }))
}

pub async fn list_user_sessions(
    State(state): State<AppState>,
    AdminAuth(actor): AdminAuth,
    Path(user_id): Path<i64>,
) -> Result<Json<SessionsListResponse>, ApiError> {
    require_target(&state, actor.role, user_id).await?;
    let sessions = queries::list_user_sessions(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(SessionsListResponse { sessions }))
}

pub async fn revoke_session(
    State(state): State<AppState>,
    AdminAuth(actor): AdminAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    // Hierarchy guard: look up the session's owner before deleting,
    // reject if the actor isn't allowed to manage that user.
    let session = queries::find_session(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    require_target(&state, actor.role, session.user_id).await?;
    queries::delete_session(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    audit(&state, actor.id, &headers, "session.revoke", id, &()).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revoke_user_sessions(
    State(state): State<AppState>,
    AdminAuth(actor): AdminAuth,
    Path(user_id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<RevokeResponse>, ApiError> {
    require_target(&state, actor.role, user_id).await?;
    let count = queries::delete_user_sessions(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?;
    audit(
        &state,
        actor.id,
        &headers,
        "session.revoke_user",
        user_id,
        &serde_json::json!({ "count": count }),
    )
    .await;
    Ok(Json(RevokeResponse { revoked: count }))
}

#[derive(Debug, Serialize)]
pub struct RevokeResponse {
    pub revoked: u64,
}

/// Admin: wipe a user's TOTP enrollment + recovery codes. The user is
/// emailed nothing — admins typically only run this after a user has
/// directly asked because they lost their device. Login proceeds as
/// password-only until the user re-enrolls.
/// Admin: clear the in-memory login-attempt tracker for a user. Used
/// to unlock a user who got progressively backoff-locked out (e.g.
/// fat-fingered their password 6+ times). Doesn't change the user's
/// password — they just get to try again immediately. The matching
/// 2FA attempt key is also cleared for users with 2FA enabled.
pub async fn unlock_login_attempts(
    State(state): State<AppState>,
    AdminAuth(actor): AdminAuth,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let user = require_target(&state, actor.role, user_id).await?;
    // Same key shape the login handler uses (lowercase username).
    let pwd_key = user.username.trim().to_lowercase();
    state.login_attempts.clear(&pwd_key).await;
    // Plus the 2FA-specific bucket keyed by user id.
    let totp_key = format!("2fa:{user_id}");
    state.login_attempts.clear(&totp_key).await;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "user.login_attempts.unlock".into(),
            target_kind: Some("user".into()),
            target_id: Some(user_id.to_string()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reset_user_totp(
    State(state): State<AppState>,
    AdminAuth(actor): AdminAuth,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    require_target(&state, actor.role, user_id).await?;
    let removed = queries::delete_user_totp(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "user.2fa.reset".into(),
            target_kind: Some("user".into()),
            target_id: Some(user_id.to_string()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    // Notify the OTHER admins so the action is visible across the team
    // (the actor doesn't need a notification of their own action).
    let actor_user = queries::find_user_by_id(&state.pool, actor.id)
        .await
        .ok()
        .flatten();
    let target_user = queries::find_user_by_id(&state.pool, user_id)
        .await
        .ok()
        .flatten();
    if let (Some(actor), Some(target)) = (actor_user, target_user) {
        crate::notifier::notify_two_factor_reset(&state, &actor, &target).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ─── Admin-triggered password reset ────────────────────────────────────────
//
// Mirrors the self-service flow in api/auth.rs (`request_password_reset`)
// but targets a specific user by ID and is gated to owners. Generates a
// single-use token, persists its hash, and emails the user a link to
// choose a new password. The admin never sees the token. Same email
// template as the self-service path so the user receives an identical
// experience whether they asked for the reset themselves or an admin
// triggered it on their behalf.
//
// Refuses silently if the user has no email on file — there's no way
// to deliver the link without surfacing the token in the admin UI,
// which would defeat the single-use guarantee.

const PASSWORD_RESET_TTL_S: i64 = 60 * 60;

#[derive(Debug, Serialize)]
pub struct PasswordResetResponse {
    pub ok: bool,
    /// Human-readable result the admin UI surfaces as a toast:
    /// "email sent", "no email on file", "SMTP not configured", etc.
    pub message: String,
}

pub async fn send_password_reset(
    State(state): State<AppState>,
    AdminAuth(actor): AdminAuth,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<PasswordResetResponse>, ApiError> {
    let user = require_target(&state, actor.role, user_id).await?;

    let Some(email) = user.email.as_deref().filter(|e| !e.trim().is_empty()) else {
        // Audit even when we can't deliver, so the admin's intent is on
        // record (they may follow up with a manual notification).
        let user_agent = headers
            .get(USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        audit_log(
            &state,
            NewAuditEntry {
                actor_user_id: Some(actor.id),
                action: "user.password_reset.skipped_no_email".into(),
                target_kind: Some("user".into()),
                target_id: Some(user_id.to_string()),
                payload_json: None,
                ip: None,
                user_agent,
            },
        )
        .await;
        return Ok(Json(PasswordResetResponse {
            ok: false,
            message: format!(
                "@{} has no email on file. Ask them to set one under Account → Profile, then retry.",
                user.username,
            ),
        }));
    };

    // Generate token + hash, persist hash only. Mirrors the self-service
    // path so consume_password_reset accepts either origin.
    let mut buf = [0u8; 32];
    crate::auth::password::fill_random(&mut buf).map_err(ApiError::Internal)?;
    let token = hex::encode(buf);
    let token_hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
    let expires_at = chimpflix_common::now_ms() + PASSWORD_RESET_TTL_S * 1000;
    let ip_str = ip.to_string();
    let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    queries::create_password_reset_token(
        &state.pool,
        user.id,
        &token_hash,
        expires_at,
        Some(ip_str.as_str()),
        user_agent,
    )
    .await
    .map_err(ApiError::Internal)?;

    let settings = state.settings.read().await.clone();
    let public_url = settings
        .public_url
        .clone()
        .map(|s| s.trim_end_matches('/').to_string());
    let reset_url = public_url
        .as_deref()
        .map(|base| format!("{base}/reset-password?token={token}"));

    let mailer_opt = Mailer::from_settings(&settings, &state.pool, &state.vault)
        .await
        .map_err(ApiError::Internal)?;

    let ua_owned = user_agent.map(str::to_owned);
    let audit_target = Some(user_id.to_string());

    let response = if let Some(mailer) = mailer_opt {
        // Build the body via the shared mail_template so the look-and-feel
        // matches the self-service email exactly — operator never wonders
        // "wait, which path sent this one?".
        let mut body = String::new();
        body.push_str(&mail_template::section_paragraph(&format!(
            "An administrator (@{}) sent you this password reset on your behalf. \
             Choose a new password below.",
            mail_template::html_escape(&actor.username),
        )));
        if let Some(url) = reset_url.as_deref() {
            body.push_str(&mail_template::section_cta("Choose a new password", url));
        }
        body.push_str(&mail_template::section_small(
            "If the button or link doesn't work, your reset token is:",
        ));
        body.push_str(&mail_template::section_code(&token));
        body.push_str(&mail_template::section_callout(
            mail_template::CalloutKind::Info,
            "This link expires in <strong>1 hour</strong> and can only be used once.",
        ));
        let html = mail_template::render_email(mail_template::EmailOpts {
            server_name: &settings.server_name,
            eyebrow_html: "Password reset",
            headline: "Reset your password",
            body_html: &body,
            footer_note: "If you didn't expect this, contact your ChimpFlix administrator. \
                          The link expires in 1 hour even if unused.",
        });
        let mut text_body = format!(
            "An administrator (@{}) sent you this password reset on your behalf. \
             Choose a new password:\n\n",
            actor.username,
        );
        if let Some(url) = reset_url.as_deref() {
            text_body.push_str(&format!("  {url}\n\n"));
        }
        text_body.push_str(&format!(
            "If the link doesn't work, your reset token is:\n\n  {token}\n\n\
             This link expires in 1 hour and can only be used once."
        ));
        let text = mail_template::render_email_text(mail_template::EmailTextOpts {
            server_name: &settings.server_name,
            headline: "Reset your password",
            body: &text_body,
            footer_note: "If you didn't expect this, contact your ChimpFlix administrator.",
        });
        let subject = format!("Reset your {} password", settings.server_name);
        match mailer
            .send(OutgoingMessage {
                to_address: email,
                to_name: user.display_name.as_deref(),
                subject: &subject,
                html: &html,
                text: &text,
            })
            .await
        {
            Ok(()) => PasswordResetResponse {
                ok: true,
                message: format!("Reset email sent to {email}."),
            },
            Err(e) => {
                // Log the full SMTP error for the operator (lettre's
                // strings include relay hostnames and credential-
                // rejection details that don't belong in an admin
                // toast). Return a generic message to the UI.
                tracing::warn!(error = %format!("{e:#}"), user_id = user.id, "admin password-reset email send failed");
                PasswordResetResponse {
                    ok: false,
                    message: "SMTP delivery failed — check the server logs.".to_string(),
                }
            }
        }
    } else {
        // SMTP not configured. The token is still persisted (admin can
        // retry once SMTP is up and the same token will still work
        // until it expires) but we surface the misconfiguration so the
        // admin isn't left wondering why no email arrived.
        PasswordResetResponse {
            ok: false,
            message:
                "Email is not configured — set SMTP under Settings → Server → Email, then retry."
                    .to_string(),
        }
    };

    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "user.password_reset.sent".into(),
            target_kind: Some("user".into()),
            target_id: audit_target,
            payload_json: Some(format!(r#"{{"delivered":{}}}"#, response.ok)),
            ip: None,
            user_agent: ua_owned,
        },
    )
    .await;

    Ok(Json(response))
}

// ─── Account lock / unlock ─────────────────────────────────────────────────
//
// Disables (or re-enables) a user account. A locked account fails the
// /auth/login gate AFTER the password check — see api/auth.rs::login.
// Owners are never lockable: the hierarchy guard below refuses an
// admin acting on an owner, and `set_user_locked` itself rejects a lock
// on an owner regardless of actor. You also can't lock your own account
// (which would lock you out of the very surface you're using).

#[derive(Debug, Serialize)]
pub struct LockResponse {
    pub user: chimpflix_library::User,
}

pub async fn lock_user(
    State(state): State<AppState>,
    AdminAuth(actor): AdminAuth,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<LockResponse>, ApiError> {
    set_locked(&state, &actor, &headers, user_id, true).await
}

pub async fn unlock_user(
    State(state): State<AppState>,
    AdminAuth(actor): AdminAuth,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<LockResponse>, ApiError> {
    set_locked(&state, &actor, &headers, user_id, false).await
}

async fn set_locked(
    state: &AppState,
    actor: &AuthUser,
    headers: &HeaderMap,
    user_id: i64,
    locked: bool,
) -> Result<Json<LockResponse>, ApiError> {
    if user_id == actor.id {
        return Err(ApiError::validation("cannot lock your own account"));
    }
    // Hierarchy guard — admins can't touch owners. require_target loads
    // the target and enforces can_act_on; the owner can never be locked.
    let target = require_target(state, actor.role, user_id).await?;
    if locked && matches!(target.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::validation("owners cannot be locked"));
    }
    let user = queries::set_user_locked(&state.pool, user_id, locked)
        .await
        .map_err(|e| ApiError::validation(format!("{e:#}")))?
        .ok_or(ApiError::NotFound)?;
    let action = if locked {
        "user.lock"
    } else {
        "user.unlock"
    };
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: action.into(),
            target_kind: Some("user".into()),
            target_id: Some(user_id.to_string()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(Json(LockResponse { user }))
}

#[derive(Debug, Serialize)]
pub struct AccessMatrixResponse {
    pub entries: Vec<AccessMatrixEntry>,
}

pub async fn get_access_matrix(
    State(state): State<AppState>,
    _admin: AdminAuth,
) -> Result<Json<AccessMatrixResponse>, ApiError> {
    let entries = queries::access_matrix(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(AccessMatrixResponse { entries }))
}

#[derive(Debug, Deserialize)]
pub struct AccessUpdate {
    /// Bulk-replace shape: per library, the desired grants. Omitted
    /// libraries are left as-is.
    pub libraries: Vec<LibraryAccessAssignment>,
}

/// Per-library grant set sent by the access matrix.
///
/// Phase 107 tri-state: when `grants` is present, each entry's `level`
/// (`view`/`full`; `none` revokes) is written verbatim via
/// `set_library_access_levels`. When only the legacy `user_ids` list is
/// sent (older clients), every listed user gets a `full` grant — preserving
/// the prior binary behaviour with no regression.
#[derive(Debug, Deserialize, Serialize)]
pub struct LibraryAccessAssignment {
    pub library_id: i64,
    /// Legacy binary shape — every listed user is granted `full`. Used when
    /// `grants` is omitted.
    #[serde(default)]
    pub user_ids: Vec<i64>,
    /// Tri-state shape — explicit per-user level. Takes precedence over
    /// `user_ids` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grants: Option<Vec<UserAccessGrant>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UserAccessGrant {
    pub user_id: i64,
    pub level: AccessLevel,
}

pub async fn put_access_matrix(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<AccessUpdate>,
) -> Result<Json<AccessMatrixResponse>, ApiError> {
    for assignment in &input.libraries {
        match &assignment.grants {
            Some(grants) => {
                let levels: Vec<(i64, AccessLevel)> =
                    grants.iter().map(|g| (g.user_id, g.level)).collect();
                queries::set_library_access_levels(&state.pool, assignment.library_id, &levels)
                    .await
                    .map_err(ApiError::Internal)?;
            }
            None => {
                // Legacy clients: binary list → full grants.
                queries::set_library_user_ids(
                    &state.pool,
                    assignment.library_id,
                    &assignment.user_ids,
                )
                .await
                .map_err(ApiError::Internal)?;
            }
        }
    }
    audit(
        &state,
        actor.id,
        &headers,
        "access.matrix.update",
        0,
        &input.libraries,
    )
    .await;
    let entries = queries::access_matrix(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(AccessMatrixResponse { entries }))
}

async fn audit<T: Serialize>(
    state: &AppState,
    actor_id: i64,
    headers: &HeaderMap,
    action: &str,
    target_id: i64,
    payload: &T,
) {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        state,
        NewAuditEntry {
            actor_user_id: Some(actor_id),
            action: action.to_string(),
            target_kind: Some("user_admin".into()),
            target_id: if target_id == 0 {
                None
            } else {
                Some(target_id.to_string())
            },
            payload_json: serde_json::to_string(payload).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;
}
