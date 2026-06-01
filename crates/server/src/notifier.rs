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
pub const KIND_JOB_FAILED: &str = "job.failed";

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

    // Per-owner: load the user, apply their notification prefs (per-kind
    // mute + quiet hours), then insert the bell row and/or send email. We
    // DO NOT short-circuit on the first failure — one owner's missing email
    // shouldn't stop another from being notified.
    let now = chimpflix_common::now_ms();
    for owner_id in owner_ids {
        let user = match queries::find_user_by_id(&state.pool, owner_id).await {
            Ok(Some(u)) => u,
            Ok(None) => continue,
            Err(e) => {
                warn!(error = %format!("{e:#}"), owner_id, "find_user_by_id during notify");
                continue;
            }
        };
        let delivery = delivery_for(&user, kind, now);
        if delivery.bell
            && let Err(e) =
                queries::insert_notification(&state.pool, owner_id, kind, &payload_json).await
        {
            warn!(error = %format!("{e:#}"), owner_id, kind, "insert_notification");
        }
        if delivery.email {
            send_email(state, &user, subject, body_text, body_html).await;
        }
    }
}

/// Which channels a notification should reach a user on, after applying
/// their per-kind preferences + quiet hours.
struct Delivery {
    bell: bool,
    email: bool,
}

/// Security events (2FA changes) can never be muted — the email templates
/// promise as much. Everything else honors `notification_prefs_json`.
fn is_security_kind(kind: &str) -> bool {
    kind == KIND_USER_TWO_FACTOR_DISABLED || kind == KIND_USER_TWO_FACTOR_RESET
}

/// Resolve per-kind delivery for `user`. The bell always records when the
/// kind is enabled (it's passive — seen when the menu opens); quiet hours
/// only hold back the interruptive email channel.
fn delivery_for(user: &User, kind: &str, now_ms: i64) -> Delivery {
    if is_security_kind(kind) {
        return Delivery {
            bell: true,
            email: user.notify_via_email,
        };
    }
    let prefs: serde_json::Value =
        serde_json::from_str(&user.notification_prefs_json).unwrap_or(serde_json::Value::Null);
    let kp = prefs.get(kind);
    let getf = |key: &str| kp.and_then(|k| k.get(key));
    let enabled = getf("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    if !enabled {
        return Delivery {
            bell: false,
            email: false,
        };
    }
    let email_pref = getf("email").and_then(|v| v.as_bool()).unwrap_or(true);
    let email = user.notify_via_email && email_pref && !in_quiet_hours(kp, now_ms);
    Delivery { bell: true, email }
}

/// True when `now` falls inside the user's quiet-hours window for this
/// kind. Hours are UTC 0–23; start > end wraps past midnight (22→7
/// suppresses 22:00–06:59). Absent/equal bounds = no quiet hours.
fn in_quiet_hours(kp: Option<&serde_json::Value>, now_ms: i64) -> bool {
    let Some(kp) = kp else { return false };
    let (Some(start), Some(end)) = (
        kp.get("quiet_start_hour").and_then(|v| v.as_i64()),
        kp.get("quiet_end_hour").and_then(|v| v.as_i64()),
    ) else {
        return false;
    };
    if !(0..24).contains(&start) || !(0..24).contains(&end) || start == end {
        return false;
    }
    let hour = (now_ms / 3_600_000).rem_euclid(24);
    if start < end {
        hour >= start && hour < end
    } else {
        hour >= start || hour < end
    }
}

async fn send_email(
    state: &AppState,
    user: &User,
    subject: &str,
    body_text: &str,
    body_html: &str,
) {
    let Some(addr) = user.email.as_deref() else {
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
            to_address: addr,
            to_name: user.display_name.as_deref(),
            subject,
            html: body_html,
            text: body_text,
        })
        .await
    {
        warn!(error = %format!("{e:#}"), user_id = user.id, "notification email send failed");
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

#[derive(Debug, Serialize)]
pub struct JobFailedPayload<'a> {
    pub job_id: i64,
    pub kind: &'a str,
    pub display_name: &'a str,
    pub error_class: Option<&'a str>,
    pub last_error: &'a str,
    pub attempts: i64,
    pub max_attempts: i64,
}

/// Fan-out when a job lands in terminal `dead` state. Only fires on
/// the dead-transition edge (retries don't notify) so a long-running
/// backfill with transient errors doesn't spam owner inboxes. Sender
/// is the worker loop; this function is best-effort and never fails
/// the surrounding handler.
pub async fn notify_job_failed(state: &AppState, payload: JobFailedPayload<'_>) {
    let server_name = state.settings.read().await.server_name.clone();
    let class_label = payload.error_class.unwrap_or("unknown");
    let subject = format!(
        "{kind} job failed ({class})",
        kind = payload.display_name,
        class = class_label,
    );
    let attempts_text = if payload.max_attempts > 0 {
        format!("{} of {} attempts", payload.attempts, payload.max_attempts)
    } else {
        format!("{} attempt(s)", payload.attempts)
    };
    let text_body = format!(
        "A {kind} job (id {id}) gave up after {attempts}.\n\
         Class: {class}\n\
         Last error: {err}\n\n\
         The job will not retry automatically. Re-queue from the admin tasks page \
         once the underlying issue is fixed.",
        kind = payload.display_name,
        id = payload.job_id,
        attempts = attempts_text,
        class = class_label,
        err = payload.last_error,
    );
    let text = mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name: &server_name,
        headline: &format!("{} job failed", payload.display_name),
        body: &text_body,
        footer_note: "You're receiving this as a ChimpFlix server owner. Job-failure alerts \
                      can be muted from Settings → Account → Notifications.",
    });

    let mut html_body = String::new();
    let display_safe = mail_template::html_escape(payload.display_name);
    let err_safe = mail_template::html_escape(payload.last_error);
    html_body.push_str(&mail_template::section_paragraph(&format!(
        "A <strong>{display_safe}</strong> job (id <code>{id}</code>) gave up after {attempts}.",
        id = payload.job_id,
        attempts = mail_template::html_escape(&attempts_text),
    )));
    let attempts_str = attempts_text.clone();
    let job_id_str = payload.job_id.to_string();
    html_body.push_str(&mail_template::section_kv(&[
        ("Job kind", payload.kind),
        ("Job id", &job_id_str),
        ("Error class", class_label),
        ("Attempts", &attempts_str),
    ]));
    html_body.push_str(&mail_template::section_callout(
        CalloutKind::Warn,
        &format!("<strong>Last error:</strong> {err_safe}"),
    ));
    html_body.push_str(&mail_template::section_cta_minimal(
        "Review failed jobs",
        "/settings/admin/library/scheduled-tasks/activity",
    ));
    let eyebrow = format!(
        "Admin · Job failure &nbsp;{}",
        mail_template::section_pip(PipKind::Warn, class_label),
    );
    let html = mail_template::render_email(mail_template::EmailOpts {
        server_name: &server_name,
        eyebrow_html: &eyebrow,
        headline: &format!("{} job failed.", payload.display_name),
        body_html: &html_body,
        footer_note: "You're receiving this as a ChimpFlix server owner. Job-failure alerts \
                      can be muted from Settings → Account → Notifications.",
    });
    notify_admins(state, KIND_JOB_FAILED, &payload, &subject, &text, &html).await;
}
