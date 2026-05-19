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
        // Branded payload so the test arrives styled identically to real
        // delivery — operators can verify the visual look-and-feel and the
        // pipeline in one round trip.
        let host_port = format!(
            "{host}:{port}",
            host = settings.email_smtp_host.as_deref().unwrap_or("?"),
            port = settings
                .email_smtp_port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "?".into()),
        );
        let security = settings
            .email_smtp_security
            .as_deref()
            .unwrap_or("?")
            .to_uppercase();
        let from = settings
            .email_from_address
            .as_deref()
            .unwrap_or("(unconfigured)");
        let when = crate::mail_template::format_email_datetime(chimpflix_common::now_ms());

        let test_html = {
            let mut body = String::new();
            body.push_str(&crate::mail_template::section_paragraph(
                "This is a test email from your ChimpFlix server. If you're reading this, the \
                 credentials and host/port combo under <strong>Settings → Server → Email</strong> \
                 are valid and outbound mail will reach your users.",
            ));
            body.push_str(&crate::mail_template::section_kv(&[
                ("Server", &host_port),
                ("Security", &security),
                ("From", from),
                ("Sent", &when),
            ]));
            body.push_str(&crate::mail_template::section_small(
                "This message was triggered by the <em>Send test</em> button in the admin panel. \
                 No user-visible email was queued.",
            ));
            crate::mail_template::render_email(crate::mail_template::EmailOpts {
                server_name: &settings.server_name,
                eyebrow_html: "SMTP test",
                headline: "Your SMTP delivery is working.",
                body_html: &body,
                footer_note: "You can safely delete this email.",
            })
        };

        let test_text = crate::mail_template::render_email_text(crate::mail_template::EmailTextOpts {
            server_name: &settings.server_name,
            headline: "SMTP delivery is working",
            body: &format!(
                "This is a test email from your ChimpFlix server. If you're reading this, the \
                 credentials and host/port combo under Settings → Server → Email are valid and \
                 outbound mail will reach your users.\n\n\
                 Server: {host_port}\n\
                 Security: {security}\n\
                 From: {from}\n\
                 Sent: {when}",
            ),
            footer_note: "You can safely delete this email.",
        });

        mailer
            .send(OutgoingMessage {
                to_address: addr,
                to_name: None,
                subject: "ChimpFlix SMTP test",
                html: &test_html,
                text: &test_text,
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
