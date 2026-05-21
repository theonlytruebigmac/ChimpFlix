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

use crate::mail_template;
use crate::mail_template::{CalloutKind, PipKind};
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

pub fn render_user_registered(
    server_name: &str,
    payload: &UserRegisteredPayload<'_>,
) -> (String, String, String) {
    let display = payload.display_name.unwrap_or(payload.username);
    let subject = format!("New user joined: {display}");
    // ── Plain text ──
    let mut text_body = format!(
        "{display} (@{u}) accepted their invite and finished signup.\n",
        u = payload.username
    );
    if let Some(email) = payload.email {
        text_body.push_str(&format!("Their email is {email}.\n"));
    }
    text_body.push_str(
        "\nThey don't have access to any libraries yet. Grant access from \
         Settings → Users → Access.",
    );
    let text = mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name,
        headline: &format!("{display} just joined your server"),
        body: &text_body,
        footer_note: "You're receiving this as a ChimpFlix server owner. \
                      Admin alerts can be muted per-kind from Settings → Account → Notifications.",
    });
    // ── HTML ──
    let display_safe = mail_template::html_escape(display);
    let user_safe = mail_template::html_escape(payload.username);
    let mut html_body = String::new();
    html_body.push_str(&mail_template::section_paragraph(&format!(
        "<strong>{display_safe}</strong> (@{user_safe}) accepted their invite and finished signup. \
         They don't have access to any libraries yet — grant them access from the admin panel."
    )));
    let mut rows: Vec<(&str, &str)> = vec![("Username", payload.username)];
    if let Some(email) = payload.email {
        rows.push(("Email", email));
    }
    if let Some(inv) = payload.invite_email {
        rows.push(("Invited via", inv));
    }
    html_body.push_str(&mail_template::section_kv(&rows));
    html_body.push_str(&mail_template::section_cta_minimal(
        "Grant library access",
        "/settings/admin/users/access",
    ));
    html_body.push_str(&mail_template::section_small(
        "Or jump to Settings → Users → Access.",
    ));
    let html = mail_template::render_email(mail_template::EmailOpts {
        server_name,
        eyebrow_html: "Admin · New signup",
        headline: &format!("{display} just joined your server."),
        body_html: &html_body,
        footer_note: "You're receiving this as a ChimpFlix server owner. \
                      Admin alerts can be muted per-kind from Settings → Account → Notifications.",
    });
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
    let server_name = state.settings.read().await.server_name.clone();
    let (subject, text, html) = render_user_registered(&server_name, &payload);
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
    let display_safe = mail_template::html_escape(display);
    let user_safe = mail_template::html_escape(&user.username);
    let server_name = state.settings.read().await.server_name.clone();
    let when = mail_template::format_email_datetime(chimpflix_common::now_ms());

    let subject = format!("{display} turned off two-factor");

    let text_body = format!(
        "{display} (@{u}) disabled their two-factor authentication.\n\
         When: {when}\n\n\
         Heads up: their account is now protected by password only. \
         If this wasn't expected, reach out to confirm — and reset \
         their password from the admin panel if you suspect compromise.",
        u = user.username,
    );
    let text = mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name: &server_name,
        headline: "Two-factor turned off",
        body: &text_body,
        footer_note: "You're receiving this as a ChimpFlix server owner. Security events can't be muted.",
    });

    let mut html_body = String::new();
    html_body.push_str(&mail_template::section_paragraph(&format!(
        "<strong>{display_safe}</strong> (@{user_safe}) disabled two-factor authentication on their account."
    )));
    html_body.push_str(&mail_template::section_callout(
        CalloutKind::Warn,
        "<strong>Heads up:</strong> their account is now protected by password only. \
         If this wasn't expected, reach out to confirm — and reset their password from \
         the admin panel if you suspect compromise.",
    ));
    html_body.push_str(&mail_template::section_kv(&[
        ("When", &when),
        ("User", display),
        ("Action", "Disabled their own 2FA"),
    ]));
    html_body.push_str(&mail_template::section_cta_minimal(
        "Review user",
        "/settings/admin/users/users",
    ));
    let eyebrow = format!(
        "Admin · Security event &nbsp;{}",
        mail_template::section_pip(PipKind::Warn, "2FA"),
    );
    let html = mail_template::render_email(mail_template::EmailOpts {
        server_name: &server_name,
        eyebrow_html: &eyebrow,
        headline: "Two-factor turned off.",
        body_html: &html_body,
        footer_note: "You're receiving this as a ChimpFlix server owner. Security events can't be muted.",
    });
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
    let ad_safe = mail_template::html_escape(actor_display);
    let au_safe = mail_template::html_escape(&actor.username);
    let td_safe = mail_template::html_escape(target_display);
    let tu_safe = mail_template::html_escape(&target_user.username);
    let server_name = state.settings.read().await.server_name.clone();
    let when = mail_template::format_email_datetime(chimpflix_common::now_ms());

    let subject = format!("{actor_display} reset 2FA for {target_display}");

    let text_body = format!(
        "{actor_display} (@{au}) reset two-factor for {target_display} (@{tu}).\n\
         When: {when}\n\n\
         The target user's account will prompt them to enroll a new TOTP authenticator on \
         next login. Recovery codes have been invalidated.",
        au = actor.username,
        tu = target_user.username,
    );
    let text = mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name: &server_name,
        headline: "Two-factor reset",
        body: &text_body,
        footer_note: "This notification fans out to every server owner so a second admin can \
                      countersign the action if needed.",
    });

    let mut html_body = String::new();
    html_body.push_str(&mail_template::section_paragraph(&format!(
        "<strong>{ad_safe}</strong> (@{au_safe}) reset two-factor authentication for \
         <strong>{td_safe}</strong> (@{tu_safe}). Their account will prompt to enroll a new TOTP \
         authenticator on next login."
    )));
    html_body.push_str(&mail_template::section_callout(
        CalloutKind::Info,
        "<strong>Recovery codes have been invalidated.</strong> If the user regenerates them \
         after re-enrollment, they'll get a fresh set.",
    ));
    html_body.push_str(&mail_template::section_kv(&[
        ("When", &when),
        ("Target user", target_display),
        ("Acted by", actor_display),
    ]));
    let eyebrow = format!(
        "Admin · Security event &nbsp;{}",
        mail_template::section_pip(PipKind::Default, "2FA reset"),
    );
    let html = mail_template::render_email(mail_template::EmailOpts {
        server_name: &server_name,
        eyebrow_html: &eyebrow,
        headline: "Two-factor reset.",
        body_html: &html_body,
        footer_note: "This notification fans out to every server owner so a second admin can \
                      countersign the action if needed.",
    });

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
