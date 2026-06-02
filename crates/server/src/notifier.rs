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
/// New content kinds. NON-security, user-tunable (per-kind enabled/email +
/// quiet hours all apply). Delivered per-user via [`notify_users`] to the
/// set of users who can see the library, not fanned out to owners.
pub const KIND_NEW_EPISODE: &str = "content.new_episode";
pub const KIND_NEW_MOVIE: &str = "content.new_movie";

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
        if delivery.discord {
            send_discord(state, &user, kind, subject, body_text).await;
        }
    }
}

/// Fan-out to a SPECIFIC set of users. Mirror of [`notify_admins`] but the
/// recipient list is supplied by the caller (e.g. the set of users who can
/// see a library) instead of being "every owner". Each recipient gets one
/// bell row + optional email/discord after their own per-kind prefs +
/// quiet hours are applied via [`delivery_for`].
///
/// Best-effort and order-independent: a missing user / unreachable email
/// for one recipient never holds back another. De-duplicates `user_ids`
/// so a user reachable via two access paths isn't notified twice.
pub async fn notify_users(
    state: &AppState,
    user_ids: &[i64],
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
    let now = chimpflix_common::now_ms();
    let mut seen = std::collections::HashSet::new();
    for &user_id in user_ids {
        if !seen.insert(user_id) {
            continue;
        }
        let user = match queries::find_user_by_id(&state.pool, user_id).await {
            Ok(Some(u)) => u,
            Ok(None) => continue,
            Err(e) => {
                warn!(error = %format!("{e:#}"), user_id, "find_user_by_id during notify_users");
                continue;
            }
        };
        let delivery = delivery_for(&user, kind, now);
        if delivery.bell
            && let Err(e) =
                queries::insert_notification(&state.pool, user_id, kind, &payload_json).await
        {
            warn!(error = %format!("{e:#}"), user_id, kind, "insert_notification");
        }
        if delivery.email {
            send_email(state, &user, subject, body_text, body_html).await;
        }
        if delivery.discord {
            send_discord(state, &user, kind, subject, body_text).await;
        }
    }
}

/// Which channels a notification should reach a user on, after applying
/// their per-kind preferences + quiet hours.
struct Delivery {
    bell: bool,
    email: bool,
    /// Mirror to the user's personal Discord webhook. Only ever true when
    /// they have one configured; otherwise treated like the email channel
    /// (security kinds always; everything else honors quiet hours).
    discord: bool,
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
    let has_discord = user.discord_webhook_url.is_some();
    if is_security_kind(kind) {
        return Delivery {
            bell: true,
            email: user.notify_via_email,
            // Push channel — like email, security kinds can't be muted.
            discord: has_discord,
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
            discord: false,
        };
    }
    let quiet = in_quiet_hours(kp, &user.timezone, now_ms);
    let email_pref = getf("email").and_then(|v| v.as_bool()).unwrap_or(true);
    let email = user.notify_via_email && email_pref && !quiet;
    // Discord is a push channel: same per-kind `enabled` gate as the bell
    // (already checked above) and the same quiet-hours suppression as email.
    let discord = has_discord && !quiet;
    Delivery {
        bell: true,
        email,
        discord,
    }
}

/// True when `now` falls inside the user's quiet-hours window for this
/// kind, evaluated against the wall clock in the user's `timezone`.
///
/// Bounds are read as minutes-since-midnight in this precedence order:
///   1. `quiet_start` / `quiet_end` strings in `"HH:MM"` form (new format).
///   2. Legacy integer `quiet_start_hour` / `quiet_end_hour` (0–23),
///      interpreted as `HH:00` — kept so existing rows keep working until
///      they're re-saved through the new UI.
/// `start > end` wraps past midnight (`22:00→07:00` suppresses
/// 22:00–06:59). Absent/equal/invalid bounds = no quiet hours.
///
/// `tz` is an IANA name; an unparseable value falls back to UTC.
fn in_quiet_hours(kp: Option<&serde_json::Value>, tz: &str, now_ms: i64) -> bool {
    let Some(kp) = kp else { return false };
    let (Some(start), Some(end)) = (quiet_bound(kp, "quiet_start", "quiet_start_hour"),
        quiet_bound(kp, "quiet_end", "quiet_end_hour"))
    else {
        return false;
    };
    if !(0..1440).contains(&start) || !(0..1440).contains(&end) || start == end {
        return false;
    }
    let now_minutes = local_minutes_of_day(tz, now_ms);
    if start < end {
        now_minutes >= start && now_minutes < end
    } else {
        // Wraps past midnight.
        now_minutes >= start || now_minutes < end
    }
}

/// Resolve one quiet-hours bound to minutes-since-midnight. Prefers the
/// new `"HH:MM"` string key; falls back to the legacy integer hour key.
/// Returns `None` when neither is present or parseable.
fn quiet_bound(kp: &serde_json::Value, hhmm_key: &str, hour_key: &str) -> Option<i64> {
    if let Some(s) = kp.get(hhmm_key).and_then(|v| v.as_str())
        && let Some(m) = parse_hhmm(s)
    {
        return Some(m);
    }
    // Legacy: integer hour 0–23 → minutes.
    kp.get(hour_key)
        .and_then(|v| v.as_i64())
        .filter(|h| (0..24).contains(h))
        .map(|h| h * 60)
}

/// Parse `"HH:MM"` (24-hour) into minutes-since-midnight `[0,1440)`.
/// Lenient on leading zeros (`"7:05"` ok); rejects out-of-range.
fn parse_hhmm(s: &str) -> Option<i64> {
    let (h_str, m_str) = s.trim().split_once(':')?;
    let h: i64 = h_str.parse().ok()?;
    let m: i64 = m_str.parse().ok()?;
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    Some(h * 60 + m)
}

/// Minutes-since-local-midnight for `now_ms` in IANA zone `tz`. Unparseable
/// zone names fall back to UTC (matches the column default + the model's
/// tolerant read), so a corrupt value can never panic the notifier.
fn local_minutes_of_day(tz: &str, now_ms: i64) -> i64 {
    use chrono::Timelike;
    let utc = chrono::DateTime::from_timestamp_millis(now_ms).unwrap_or_default();
    let zone: chrono_tz::Tz = tz.parse().unwrap_or(chrono_tz::UTC);
    let local = utc.with_timezone(&zone);
    i64::from(local.hour() * 60 + local.minute())
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

/// Discord embed brand color (Discord blurple). Plain decimal int as the
/// Discord webhook API expects for the `color` field.
const DISCORD_EMBED_COLOR: i64 = 0x5865F2;

/// POST a notification to the user's personal Discord webhook as an embed.
/// Best-effort: builds a fresh short-timeout client, logs + swallows every
/// error, and never blocks the surrounding request. `title` is the
/// notification subject; `body` is the plain-text body (Discord embed
/// descriptions render markdown, and the plain-text variant is the closest
/// match — the HTML body would show raw tags).
async fn send_discord(_state: &AppState, user: &User, kind: &str, title: &str, body: &str) {
    let Some(url) = user.discord_webhook_url.as_deref() else {
        return;
    };
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        // Host validation pins the first hop to discord.com; disabling
        // redirects keeps a webhook URL from bouncing the POST to an
        // internal address (SSRF defense-in-depth).
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, user_id = user.id, kind, "discord client build");
            return;
        }
    };
    // Discord embed descriptions are capped at 4096 chars; titles at 256.
    // Truncate defensively so an oversized body doesn't get the whole POST
    // rejected with a 400.
    let title_trunc: String = title.chars().take(256).collect();
    let body_trunc: String = body.chars().take(4096).collect();
    let payload = serde_json::json!({
        "embeds": [{
            "title": title_trunc,
            "description": body_trunc,
            "color": DISCORD_EMBED_COLOR,
        }]
    });
    match client.post(url).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => {
            warn!(
                status = resp.status().as_u16(),
                user_id = user.id,
                kind,
                "discord webhook returned non-success"
            );
        }
        Err(e) => {
            // `.without_url()` strips the request URL from the error before
            // logging — the webhook URL carries the secret token, and
            // reqwest's Display otherwise appends "for url (...)".
            warn!(
                error = %e.without_url(),
                user_id = user.id,
                kind,
                "discord webhook send failed"
            );
        }
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

// ---------------------------------------------------------------------------
// Content notifications — NEW_EPISODE / NEW_MOVIE.
//
// These are NON-security, user-tunable, and delivered per-user via
// [`notify_users`] to the audience the caller resolves (e.g. the users who
// can see the library, or who follow a show). The render helpers below own
// the template knowledge; the (subject, text, html) tuple they return is
// fed straight into `notify_users`. A background scan-completion job is the
// intended caller so the scan hot-path stays clear.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct NewEpisodePayload<'a> {
    /// `items.id` of the parent show.
    pub show_id: i64,
    pub show_title: &'a str,
    pub season_number: i32,
    pub episode_number: i32,
    /// Episode title, when known.
    pub episode_title: Option<&'a str>,
}

pub fn render_new_episode(
    server_name: &str,
    payload: &NewEpisodePayload<'_>,
) -> (String, String, String) {
    let code = format!(
        "S{:02}E{:02}",
        payload.season_number.max(0),
        payload.episode_number.max(0),
    );
    let ep_suffix = payload
        .episode_title
        .map(|t| format!(" — {t}"))
        .unwrap_or_default();
    let subject = format!("New episode of {}: {code}", payload.show_title);

    // ── Plain text ──
    let text_body = format!(
        "A new episode of {show} is available: {code}{ep}.\n\n\
         It's ready to watch now.",
        show = payload.show_title,
        ep = ep_suffix,
    );
    let text = mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name,
        headline: &format!("New episode of {}", payload.show_title),
        body: &text_body,
        footer_note: "You're receiving this because new-episode alerts are on. \
                      Mute them from Settings → Account → Notifications.",
    });

    // ── HTML ──
    let show_safe = mail_template::html_escape(payload.show_title);
    let code_safe = mail_template::html_escape(&code);
    let ep_safe = payload
        .episode_title
        .map(mail_template::html_escape)
        .map(|t| format!(" — {t}"))
        .unwrap_or_default();
    let mut html_body = String::new();
    html_body.push_str(&mail_template::section_paragraph(&format!(
        "A new episode of <strong>{show_safe}</strong> is available: \
         <strong>{code_safe}</strong>{ep_safe}."
    )));
    // Deep-link to the title modal on the home page — the canonical
    // detail entry point (`/?title=<item_id>`), which resolves for both
    // shows and movies. The show's parent item id opens the show modal.
    html_body.push_str(&mail_template::section_cta(
        "Watch now",
        &format!("/?title={}", payload.show_id),
    ));
    let html = mail_template::render_email(mail_template::EmailOpts {
        server_name,
        eyebrow_html: "New episode",
        headline: &format!("New episode of {}.", payload.show_title),
        body_html: &html_body,
        footer_note: "You're receiving this because new-episode alerts are on. \
                      Mute them from Settings → Account → Notifications.",
    });
    (subject, text, html)
}

#[derive(Debug, Serialize)]
pub struct NewMoviePayload<'a> {
    /// `items.id` of the movie.
    pub item_id: i64,
    pub title: &'a str,
    pub year: Option<i32>,
}

pub fn render_new_movie(
    server_name: &str,
    payload: &NewMoviePayload<'_>,
) -> (String, String, String) {
    let year_suffix = payload
        .year
        .map(|y| format!(" ({y})"))
        .unwrap_or_default();
    let subject = format!("New movie added: {}{year_suffix}", payload.title);

    // ── Plain text ──
    let text_body = format!(
        "{title}{year} was just added to the library.\n\n\
         It's ready to watch now.",
        title = payload.title,
        year = year_suffix,
    );
    let text = mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name,
        headline: &format!("{} was added", payload.title),
        body: &text_body,
        footer_note: "You're receiving this because new-movie alerts are on. \
                      Mute them from Settings → Account → Notifications.",
    });

    // ── HTML ──
    let title_safe = mail_template::html_escape(payload.title);
    let year_safe = mail_template::html_escape(&year_suffix);
    let mut html_body = String::new();
    html_body.push_str(&mail_template::section_paragraph(&format!(
        "<strong>{title_safe}</strong>{year_safe} was just added to the library."
    )));
    // Deep-link to the title modal (`/?title=<item_id>`) — the canonical
    // detail entry point that resolves for movies.
    html_body.push_str(&mail_template::section_cta(
        "Watch now",
        &format!("/?title={}", payload.item_id),
    ));
    let html = mail_template::render_email(mail_template::EmailOpts {
        server_name,
        eyebrow_html: "New movie",
        headline: &format!("{} was added.", payload.title),
        body_html: &html_body,
        footer_note: "You're receiving this because new-movie alerts are on. \
                      Mute them from Settings → Account → Notifications.",
    });
    (subject, text, html)
}

// ---------------------------------------------------------------------------
// Batched / summarized content notifications.
//
// Anti-spam summaries emitted by the `notify_new_content` background job
// when a single scan adds many episodes of one show or many movies to one
// library. The per-row [`NewEpisodePayload`] / [`NewMoviePayload`] above are
// still used for the singular case; these collapse a burst into one row.
// Both reuse [`KIND_NEW_EPISODE`] / [`KIND_NEW_MOVIE`] so they honor the
// same per-user prefs + quiet hours, and the frontend can render them by
// branching on the presence of the `count` field in the payload.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct NewEpisodeBatchPayload<'a> {
    /// `items.id` of the parent show.
    pub show_id: i64,
    pub show_title: &'a str,
    /// Number of new episodes in this batch (always >= 2 — the singular
    /// case uses [`NewEpisodePayload`]).
    pub count: usize,
}

pub fn render_new_episodes_batch(
    server_name: &str,
    payload: &NewEpisodeBatchPayload<'_>,
) -> (String, String, String) {
    let n = payload.count;
    let subject = format!("{n} new episodes of {}", payload.show_title);

    // ── Plain text ──
    let text_body = format!(
        "{n} new episodes of {show} are available.\n\n\
         They're ready to watch now.",
        show = payload.show_title,
    );
    let text = mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name,
        headline: &format!("{n} new episodes of {}", payload.show_title),
        body: &text_body,
        footer_note: "You're receiving this because new-episode alerts are on. \
                      Mute them from Settings → Account → Notifications.",
    });

    // ── HTML ──
    let show_safe = mail_template::html_escape(payload.show_title);
    let mut html_body = String::new();
    html_body.push_str(&mail_template::section_paragraph(&format!(
        "<strong>{n} new episodes</strong> of <strong>{show_safe}</strong> are available."
    )));
    // Deep-link to the show modal on the home page.
    html_body.push_str(&mail_template::section_cta(
        "Watch now",
        &format!("/?title={}", payload.show_id),
    ));
    let html = mail_template::render_email(mail_template::EmailOpts {
        server_name,
        eyebrow_html: "New episodes",
        headline: &format!("{n} new episodes of {}.", payload.show_title),
        body_html: &html_body,
        footer_note: "You're receiving this because new-episode alerts are on. \
                      Mute them from Settings → Account → Notifications.",
    });
    (subject, text, html)
}

#[derive(Debug, Serialize)]
pub struct NewMovieBatchPayload<'a> {
    /// `libraries.id` the movies landed in.
    pub library_id: i64,
    pub library_name: &'a str,
    /// Number of new movies in this batch (always >= the detail threshold).
    pub count: usize,
}

pub fn render_new_movies_batch(
    server_name: &str,
    payload: &NewMovieBatchPayload<'_>,
) -> (String, String, String) {
    let n = payload.count;
    let subject = format!("{n} new movies in {}", payload.library_name);

    // ── Plain text ──
    let text_body = format!(
        "{n} new movies were added to {lib}.\n\n\
         They're ready to watch now.",
        lib = payload.library_name,
    );
    let text = mail_template::render_email_text(mail_template::EmailTextOpts {
        server_name,
        headline: &format!("{n} new movies in {}", payload.library_name),
        body: &text_body,
        footer_note: "You're receiving this because new-movie alerts are on. \
                      Mute them from Settings → Account → Notifications.",
    });

    // ── HTML ──
    let lib_safe = mail_template::html_escape(payload.library_name);
    let mut html_body = String::new();
    html_body.push_str(&mail_template::section_paragraph(&format!(
        "<strong>{n} new movies</strong> were added to <strong>{lib_safe}</strong>."
    )));
    // Deep-link to the home page (the library's content shows in its rails);
    // no single item to open for a multi-movie batch.
    html_body.push_str(&mail_template::section_cta("Browse now", "/"));
    let html = mail_template::render_email(mail_template::EmailOpts {
        server_name,
        eyebrow_html: "New movies",
        headline: &format!("{n} new movies in {}.", payload.library_name),
        body_html: &html_body,
        footer_note: "You're receiving this because new-movie alerts are on. \
                      Mute them from Settings → Account → Notifications.",
    });
    (subject, text, html)
}
