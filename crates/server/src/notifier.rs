//! Fire-and-forget notification helper.
//!
//! Feature code calls [`notify_admins`] to drop a row into every owner's
//! inbox. When a recipient has `notify_via_email = true` AND SMTP is
//! configured AND they have an email address, the same notification is
//! mirrored as an email. Failures are logged but never bubble up — the
//! request that triggered the notification mustn't fail because a
//! follow-up email was unreachable.

use chimpflix_library::{User, queries};
use serde::Serialize;
use tracing::warn;

use crate::mailer::{Mailer, OutgoingMessage};
use crate::state::AppState;

/// Stable kind discriminators. Renderers (frontend + email template)
/// dispatch on these strings, so adding a new event means: add a const
/// here, a payload struct, and the rendering side.
pub const KIND_USER_REGISTERED: &str = "user.registered";
pub const KIND_USER_TWO_FACTOR_DISABLED: &str = "user.2fa.disabled";
pub const KIND_USER_TWO_FACTOR_RESET: &str = "user.2fa.reset";

#[derive(Debug, Serialize)]
pub struct UserRegisteredPayload<'a> {
    pub user_id: i64,
    pub username: &'a str,
    pub display_name: Option<&'a str>,
    pub email: Option<&'a str>,
    pub invite_email: Option<&'a str>,
}

/// Fan-out to every owner. Each owner gets one row in `notifications` +
/// optionally one email if they opted in.
pub async fn notify_admins(
    state: &AppState,
    kind: &str,
    payload: &impl Serialize,
    subject: &str,
    body_text: &str,
    body_html: &str,
) {
    let payload_json = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, kind, "serialize notification payload");
            return;
        }
    };

    let owner_ids = match queries::list_owner_ids(&state.pool).await {
        Ok(ids) => ids,
        Err(e) => {
            warn!(error = %format!("{e:#}"), "list_owner_ids");
            return;
        }
    };

    // Insert + maybe-email each owner. We DO NOT short-circuit on the
    // first failure — one owner's missing email shouldn't stop another
    // from being notified.
    for owner_id in owner_ids {
        if let Err(e) =
            queries::insert_notification(&state.pool, owner_id, kind, &payload_json).await
        {
            warn!(error = %format!("{e:#}"), owner_id, kind, "insert_notification");
            continue;
        }
        send_email_if_opted_in(state, owner_id, subject, body_text, body_html).await;
    }
}

async fn send_email_if_opted_in(
    state: &AppState,
    owner_id: i64,
    subject: &str,
    body_text: &str,
    body_html: &str,
) {
    let user_opt = match queries::find_user_by_id(&state.pool, owner_id).await {
        Ok(u) => u,
        Err(e) => {
            warn!(error = %format!("{e:#}"), owner_id, "find_user_by_id during notify");
            return;
        }
    };
    let Some(User {
        notify_via_email: true,
        email: Some(addr),
        display_name,
        ..
    }) = user_opt
    else {
        return;
    };
    let settings = state.settings.read().await.clone();
    let mailer = match Mailer::from_settings(&settings, &state.pool, &state.vault).await {
        Ok(Some(m)) => m,
        Ok(None) => return, // SMTP not configured — silent skip
        Err(e) => {
            warn!(error = %format!("{e:#}"), "Mailer::from_settings during notify");
            return;
        }
    };
    if let Err(e) = mailer
        .send(OutgoingMessage {
            to_address: &addr,
            to_name: display_name.as_deref(),
            subject,
            html: body_html,
            text: body_text,
        })
        .await
    {
        warn!(error = %format!("{e:#}"), owner_id, "notification email send failed");
    }
}

// ---------------------------------------------------------------------------
// Pre-baked rendering helpers — keep template knowledge in one place.
// ---------------------------------------------------------------------------

pub fn render_user_registered(payload: &UserRegisteredPayload<'_>) -> (String, String, String) {
    let display = payload.display_name.unwrap_or(payload.username);
    let subject = format!("New user joined: {display}");
    let email_line = payload
        .email
        .map(|e| format!(" ({e})"))
        .unwrap_or_default();
    let text = format!(
        "{display} (@{u}){email} just accepted their invite and finished signup.\n\n\
         Grant library access from Settings → Users → Access.",
        u = payload.username,
        email = email_line,
    );
    let html = format!(
        r#"<p><strong>{display_safe}</strong> (@{user_safe}){email_safe} just accepted their invite and finished signup.</p>
           <p>Grant library access from Settings → Users → Access.</p>"#,
        display_safe = html_escape(display),
        user_safe = html_escape(payload.username),
        email_safe = payload
            .email
            .map(|e| format!(" ({})", html_escape(e)))
            .unwrap_or_default(),
    );
    (subject, text, html)
}

pub async fn notify_user_registered(state: &AppState, new_user: &User, invite_email: Option<&str>) {
    let payload = UserRegisteredPayload {
        user_id: new_user.id,
        username: &new_user.username,
        display_name: new_user.display_name.as_deref(),
        email: new_user.email.as_deref(),
        invite_email,
    };
    let (subject, text, html) = render_user_registered(&payload);
    notify_admins(
        state,
        KIND_USER_REGISTERED,
        &payload,
        &subject,
        &text,
        &html,
    )
    .await;
}

#[derive(Debug, Serialize)]
pub struct UserTwoFactorPayload<'a> {
    pub user_id: i64,
    pub username: &'a str,
}

pub async fn notify_two_factor_disabled(state: &AppState, user: &User) {
    let payload = UserTwoFactorPayload {
        user_id: user.id,
        username: &user.username,
    };
    let display = user.display_name.as_deref().unwrap_or(&user.username);
    let subject = format!("{display} turned off two-factor");
    let text = format!(
        "{display} (@{u}) disabled their two-factor authentication.\n\n\
         If you didn't expect this, reach out — and consider switching\n\
         the global enforcement to 'required' in Settings → Server → General.",
        u = user.username,
    );
    let html = format!(
        r#"<p><strong>{d_safe}</strong> (@{u_safe}) disabled their two-factor authentication.</p>
           <p>If you didn't expect this, reach out — and consider switching the global enforcement to 'required' in Settings → Server → General.</p>"#,
        d_safe = html_escape(display),
        u_safe = html_escape(&user.username),
    );
    notify_admins(
        state,
        KIND_USER_TWO_FACTOR_DISABLED,
        &payload,
        &subject,
        &text,
        &html,
    )
    .await;
}

pub async fn notify_two_factor_reset(state: &AppState, actor: &User, target_user: &User) {
    #[derive(Serialize)]
    struct Payload<'a> {
        actor_user_id: i64,
        actor_username: &'a str,
        target_user_id: i64,
        target_username: &'a str,
    }
    let payload = Payload {
        actor_user_id: actor.id,
        actor_username: &actor.username,
        target_user_id: target_user.id,
        target_username: &target_user.username,
    };
    let actor_display = actor.display_name.as_deref().unwrap_or(&actor.username);
    let target_display = target_user
        .display_name
        .as_deref()
        .unwrap_or(&target_user.username);
    let subject = format!("{actor_display} reset 2FA for {target_display}");
    let text = format!(
        "{actor_display} (@{au}) reset two-factor for {target_display} (@{tu}).\n\n\
         The user can now log in with just their password until they re-enroll.",
        au = actor.username,
        tu = target_user.username,
    );
    let html = format!(
        r#"<p><strong>{ad_safe}</strong> (@{au_safe}) reset two-factor for <strong>{td_safe}</strong> (@{tu_safe}).</p>
           <p>The user can now log in with just their password until they re-enroll.</p>"#,
        ad_safe = html_escape(actor_display),
        au_safe = html_escape(&actor.username),
        td_safe = html_escape(target_display),
        tu_safe = html_escape(&target_user.username),
    );
    notify_admins(
        state,
        KIND_USER_TWO_FACTOR_RESET,
        &payload,
        &subject,
        &text,
        &html,
    )
    .await;
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
