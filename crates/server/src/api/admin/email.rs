//! Email/SMTP admin endpoints layered on top of `/admin/settings`.
//!
//! Most SMTP config (host/port/username/security/from-address/from-name)
//! flows through the generic `/admin/settings` PATCH. This module owns
//! the bits that don't fit there:
//!
//!   * **Password** — held in the credential vault, not the settings
//!     singleton. Set/clear via dedicated endpoints so it's never
//!     returned in the settings GET (defense-in-depth — the only way
//!     to read the password is by being the SMTP relay).
//!   * **Test** — runs a HELO/EHLO + optional AUTH against the
//!     configured relay so the admin sees failures before the first
//!     real invite/reset email is queued.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use axum::http::StatusCode;
use chimpflix_library::{NewAuditEntry, queries};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::mailer::{Mailer, OutgoingMessage, SMTP_PASSWORD_SECRET};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct EmailStatusResponse {
    /// Whether all required fields (host + from address) are present.
    pub configured: bool,
    /// Whether a password is currently in the vault. We never return
    /// the password itself; this just lets the admin UI render "set"
    /// vs "not set" without prompting for re-entry.
    pub has_password: bool,
    /// Echo of the current SMTP host, port, security, username, and
    /// from-address/name — useful for read-back without joining
    /// settings GET against a separate "is configured" probe.
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i64>,
    pub smtp_username: Option<String>,
    pub smtp_security: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
}

pub async fn get_status(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<EmailStatusResponse>, ApiError> {
    let settings = state.settings.read().await.clone();
    let has_password = queries::vault_get(&state.pool, &state.vault, SMTP_PASSWORD_SECRET)
        .await
        .map_err(ApiError::Internal)?
        .is_some();
    let configured = settings
        .email_smtp_host
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
        && settings
            .email_from_address
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
    Ok(Json(EmailStatusResponse {
        configured,
        has_password,
        smtp_host: settings.email_smtp_host,
        smtp_port: settings.email_smtp_port,
        smtp_username: settings.email_smtp_username,
        smtp_security: settings.email_smtp_security,
        from_address: settings.email_from_address,
        from_name: settings.email_from_name,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SetPasswordRequest {
    /// Plaintext password. Stored encrypted-at-rest in the vault.
    pub password: String,
}

pub async fn set_password(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<SetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    if input.password.is_empty() {
        return Err(ApiError::validation("password must not be empty"));
    }
    if input.password.len() > 1024 {
        return Err(ApiError::validation(
            "password must be at most 1024 characters",
        ));
    }
    queries::vault_set(
        &state.pool,
        &state.vault,
        SMTP_PASSWORD_SECRET,
        &input.password,
        Some(actor.id),
    )
    .await
    .map_err(ApiError::Internal)?;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "settings.email.password.set".into(),
            target_kind: Some("settings".into()),
            target_id: Some("email".into()),
            // Never log the password itself — only the fact that it
            // was rotated.
            payload_json: Some(r#"{"action":"rotated"}"#.into()),
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn clear_password(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let removed = queries::vault_delete(&state.pool, SMTP_PASSWORD_SECRET)
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
            action: "settings.email.password.clear".into(),
            target_kind: Some("settings".into()),
            target_id: Some("email".into()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, Default)]
pub struct TestRequest {
    /// When set, deliver a real test email here (after the connection
    /// check passes). Omit to only do the SMTP handshake.
    #[serde(default)]
    pub send_to: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestResponse {
    pub ok: bool,
    /// Human-readable diagnostic — what was attempted. Errors surface
    /// via the normal ApiError path with sanitized messages.
    pub message: String,
}

pub async fn test(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(req): Json<TestRequest>,
) -> Result<Json<TestResponse>, ApiError> {
    let settings = state.settings.read().await.clone();
    let mailer = Mailer::from_settings(&settings, &state.pool, &state.vault)
        .await
        .map_err(|e| ApiError::validation(format!("email config rejected: {e}")))?
        .ok_or_else(|| ApiError::validation("email is not configured (set SMTP host first)"))?;

    mailer
        .test_connection()
        .await
        .map_err(|e| ApiError::validation(format!("SMTP connection failed: {e}")))?;

    let mut message = "SMTP handshake succeeded".to_string();
    if let Some(addr) = req.send_to.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // Quick syntactic sanity check before bothering the relay.
        if !addr.contains('@') || addr.len() > 320 {
            return Err(ApiError::validation("send_to must look like local@domain"));
        }
        mailer
            .send(OutgoingMessage {
                to_address: addr,
                to_name: None,
                subject: "ChimpFlix SMTP test",
                html: "<p>This is a test email from your ChimpFlix server. \
                       If you're reading this, SMTP delivery is working.</p>",
                text: "This is a test email from your ChimpFlix server. \
                       If you're reading this, SMTP delivery is working.",
            })
            .await
            .map_err(|e| ApiError::validation(format!("test email failed: {e}")))?;
        message = format!("test email sent to {addr}");
    }

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "settings.email.test".into(),
            target_kind: Some("settings".into()),
            target_id: Some("email".into()),
            payload_json: req
                .send_to
                .as_deref()
                .map(|addr| format!(r#"{{"send_to":"{addr}"}}"#)),
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(Json(TestResponse {
        ok: true,
        message,
    }))
}
