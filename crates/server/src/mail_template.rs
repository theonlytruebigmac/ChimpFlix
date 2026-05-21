//! Shared HTML/text builders for every email the server sends.
//!
//! Gmail (and most other webmail clients) strip `<style>` blocks, so the
//! email bodies have to use inline `style=""` on every element. To keep
//! the eight templates from drifting visually, each one composes a body
//! out of the section helpers below and hands the result to
//! [`render_email`], which wraps it in the shared header/footer chrome.
//!
//! The look is locked in by `docs/email-mockups.html` — when changing the
//! design, change the mockup first, then mirror here.
//!
//! Style tokens (Netflix-ish):
//!   - Background:  #000000 (header/footer/page) / #141414 (content)
//!   - Accent:      #e50914 (brand red, CTA, eyebrow underline)
//!   - Body text:   #d2d2d2 over dark
//!   - Muted text:  #8c8c8c / #6e6e6e (footer)
//!   - Brand wordmark uses Helvetica Neue; system stack falls back.

use chrono::{DateTime, Local, TimeZone};

// ─── Style tokens ──────────────────────────────────────────────────────────
//
// Inlined into every element via `style=""`. Defined as `const &str` so
// callers compose by `format!()` rather than typing the raw strings.

const FONT_STACK: &str =
    "\"Helvetica Neue\", Helvetica, Arial, sans-serif";

const STYLE_BODY: &str = "background:#000000;margin:0 auto;padding:0;max-width:600px;color:#ffffff;font-family:\"Helvetica Neue\",Helvetica,Arial,sans-serif";
const STYLE_HEADER: &str = "background:#000000;padding:28px 32px 24px;text-align:left;border-bottom:4px solid #e50914";
const STYLE_BRAND: &str = "color:#e50914;font-size:28px;font-weight:900;letter-spacing:0.04em;text-transform:uppercase;text-decoration:none;display:inline-block;line-height:1";
const STYLE_CONTENT: &str = "background:#141414;padding:36px 32px";
const STYLE_EYEBROW: &str = "color:#b3b3b3;font-size:11px;text-transform:uppercase;letter-spacing:0.2em;margin:0 0 12px;padding-top:4px";
const STYLE_HEADLINE: &str = "color:#ffffff;font-size:26px;font-weight:800;line-height:1.2;margin:0 0 16px;letter-spacing:-0.01em";
const STYLE_PARAGRAPH: &str = "color:#d2d2d2;font-size:15px;line-height:1.6;margin:0 0 18px";
const STYLE_SMALL: &str = "color:#8c8c8c;font-size:13px;line-height:1.6;margin:0 0 12px";
const STYLE_CTA_WRAP: &str = "margin:28px 0 32px";
const STYLE_CTA: &str = "display:inline-block;background:#e50914;color:#ffffff;font-weight:700;font-size:15px;padding:14px 28px;border-radius:4px;text-decoration:none;letter-spacing:0.02em";
const STYLE_LINK: &str = "color:#ffffff;word-break:break-all;text-decoration:underline";
const STYLE_CODE: &str = "display:block;background:#1f1f1f;border:1px solid #2b2b2b;color:#ffffff;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:13px;padding:14px 16px;border-radius:6px;word-break:break-all;margin:0 0 16px";
const STYLE_QUOTE: &str = "background:#1c1c1c;border-left:3px solid #404040;padding:14px 18px;margin:14px 0 22px;color:#d2d2d2;font-size:14px;line-height:1.55;white-space:pre-wrap";
const STYLE_KV: &str = "width:100%;border-collapse:collapse;margin:8px 0 24px";
const STYLE_KV_K: &str = "padding:10px 16px 10px 0;border-bottom:1px solid #2a2a2a;color:#8c8c8c;width:130px;font-size:12px;text-transform:uppercase;letter-spacing:0.08em;vertical-align:top";
const STYLE_KV_V: &str = "padding:10px 0;border-bottom:1px solid #2a2a2a;color:#d2d2d2;font-size:14px;vertical-align:top";
const STYLE_FOOTER: &str = "background:#000000;padding:28px 32px;color:#6e6e6e;font-size:12px;line-height:1.6;border-top:1px solid #1f1f1f";
const STYLE_LEGAL: &str = "color:#4a4a4a;font-size:11px;margin:10px 0 0";

// Callout border colors keyed by `CalloutKind`.
const CALLOUT_BORDER_DEFAULT: &str = "#e50914";
const CALLOUT_BORDER_INFO: &str = "#4f8cff";
const CALLOUT_BORDER_WARN: &str = "#f5a623";

// Pip background colors keyed by `PipKind`.
const PIP_BG_DEFAULT: &str = "#2a2a2a";
const PIP_BG_WARN: &str = "#f5a623";
const PIP_BG_DANGER: &str = "#e50914";

// ─── Public API ────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub enum CalloutKind {
    /// Red left-border. Use for "expires at…" / standard call-outs.
    Default,
    /// Amber left-border. Use for security warnings ("their account is
    /// now password-only").
    Warn,
    /// Blue left-border. Use for informational notes ("link expires in
    /// 1 hour" — neutral logistics).
    Info,
}

#[derive(Clone, Copy)]
pub enum PipKind {
    /// Neutral gray pill, white text.
    Default,
    /// Amber pill, black text — used for non-critical security pips
    /// like "2FA".
    Warn,
    /// Brand-red pill, white text — used for severity-tagged events.
    #[allow(dead_code)]
    Danger,
}

pub struct EmailOpts<'a> {
    /// Branded server name — the operator-configured server display
    /// name, falls back to the literal "ChimpFlix" inside [`render_email`]
    /// when empty.
    pub server_name: &'a str,
    /// Small uppercase label above the headline. Pass empty `""` to
    /// skip it. May contain HTML (e.g. a [`section_pip`] result) — the
    /// caller is responsible for escaping any dynamic text inside.
    pub eyebrow_html: &'a str,
    /// One-sentence H1. Plain text only; we HTML-escape it.
    pub headline: &'a str,
    /// Composed body HTML (already-rendered sections, concatenated).
    pub body_html: &'a str,
    /// Footer micro-copy explaining why the recipient is getting this
    /// email. Plain text; we HTML-escape it. Links in the footer should
    /// be built explicitly via [`footer_link`] and concatenated.
    pub footer_note: &'a str,
}

pub struct EmailTextOpts<'a> {
    pub server_name: &'a str,
    /// One-line headline equivalent. Appears under the brand strip.
    pub headline: &'a str,
    /// Pre-composed body lines (one per `\n`). Caller composes.
    pub body: &'a str,
    pub footer_note: &'a str,
}

/// Render the full HTML email (doctype, `<html>`, header chrome, body,
/// footer). Result is ready to hand to [`crate::mailer::OutgoingMessage::html`].
///
/// The `body_html` should be a concatenation of `section_*` helper
/// results — no need to wrap in a container, the chrome handles that.
pub fn render_email(opts: EmailOpts<'_>) -> String {
    let brand = if opts.server_name.trim().is_empty() {
        "ChimpFlix"
    } else {
        opts.server_name
    };
    let brand_safe = html_escape(brand).to_uppercase();
    let eyebrow_html = if opts.eyebrow_html.is_empty() {
        String::new()
    } else {
        format!(
            r#"<p style="{STYLE_EYEBROW}">{eyebrow}</p>"#,
            eyebrow = opts.eyebrow_html,
        )
    };
    let headline_safe = html_escape(opts.headline);
    let footer_safe = html_escape(opts.footer_note);
    format!(
        "<!doctype html>\n\
         <html><head><meta charset=\"utf-8\"/><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"/></head>\n\
         <body style=\"margin:0;padding:0;background:#000000;font-family:{FONT_STACK};\">\n\
         <table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"background:#000000;\"><tr><td align=\"center\">\n\
         <div style=\"{STYLE_BODY}\">\n\
           <div style=\"{STYLE_HEADER}\"><span style=\"{STYLE_BRAND}\">{brand_safe}</span></div>\n\
           <div style=\"{STYLE_CONTENT}\">\n\
             {eyebrow_html}\n\
             <h1 style=\"{STYLE_HEADLINE}\">{headline_safe}</h1>\n\
             {body}\n\
           </div>\n\
           <div style=\"{STYLE_FOOTER}\">\n\
             {footer_safe}\n\
             <p style=\"{STYLE_LEGAL}\">Sent by {brand_safe} · automated message, please don't reply.</p>\n\
           </div>\n\
         </div>\n\
         </td></tr></table></body></html>",
        body = opts.body_html,
    )
}

/// Render a matching plain-text email. Tries to look reasonable even
/// without a renderer that understands monospace — uses `===` rules so
/// the header/footer separators are visually obvious.
pub fn render_email_text(opts: EmailTextOpts<'_>) -> String {
    let brand = if opts.server_name.trim().is_empty() {
        "ChimpFlix"
    } else {
        opts.server_name
    };
    let bar = "═".repeat(brand.chars().count());
    let rule = "─".repeat(56);
    format!(
        "{brand_upper}\n{bar}\n\n\
         {headline}\n\n\
         {body}\n\n\
         {rule}\n{footer}\n\n\
         Sent by {brand} · automated message, please don't reply.\n",
        brand_upper = brand.to_uppercase(),
        headline = opts.headline,
        body = opts.body.trim_end(),
        footer = opts.footer_note,
    )
}

// ─── Section helpers ───────────────────────────────────────────────────────

/// One paragraph. `html` is the inner HTML — caller is responsible for
/// escaping dynamic data; literal copy can use `<strong>`/`<em>` freely.
pub fn section_paragraph(html: &str) -> String {
    format!(r#"<p style="{STYLE_PARAGRAPH}">{html}</p>"#)
}

/// Smaller, dimmer paragraph. Same HTML semantics.
pub fn section_small(html: &str) -> String {
    format!(r#"<p style="{STYLE_SMALL}">{html}</p>"#)
}

/// CTA button. Pass the plain label (auto-escaped) and the destination
/// URL (also escaped to defang attribute injection). Adds the standard
/// "Or paste this link" subline below, since email clients regularly
/// mangle anchor styling.
pub fn section_cta(label: &str, url: &str) -> String {
    let label_safe = html_escape(label);
    let url_safe = html_escape(url);
    format!(
        r#"<div style="{STYLE_CTA_WRAP}"><a href="{url_safe}" style="{STYLE_CTA}">{label_safe}</a></div>
<p style="{STYLE_SMALL}">Or paste this link into your browser:<br/><a href="{url_safe}" style="{STYLE_LINK}">{url_safe}</a></p>"#
    )
}

/// CTA without the paste-link subline — for admin "Open in admin"-style
/// buttons where the operator already has the dashboard open.
pub fn section_cta_minimal(label: &str, url: &str) -> String {
    let label_safe = html_escape(label);
    let url_safe = html_escape(url);
    format!(
        r#"<div style="{STYLE_CTA_WRAP}"><a href="{url_safe}" style="{STYLE_CTA}">{label_safe}</a></div>"#
    )
}

/// Monospace block — typically a token, code, or backup invite code.
pub fn section_code(text: &str) -> String {
    let safe = html_escape(text);
    format!(r#"<code style="{STYLE_CODE}">{safe}</code>"#)
}

/// Quoted user-submitted text (issue-report body). Preserves newlines
/// and HTML-escapes. Caller passes the raw user input.
pub fn section_quote(text: &str) -> String {
    let safe = html_escape(text);
    format!(r#"<div style="{STYLE_QUOTE}">{safe}</div>"#)
}

/// Left-rule callout. `body_html` is HTML — `<strong>` etc. ok.
pub fn section_callout(kind: CalloutKind, body_html: &str) -> String {
    let border = match kind {
        CalloutKind::Default => CALLOUT_BORDER_DEFAULT,
        CalloutKind::Warn => CALLOUT_BORDER_WARN,
        CalloutKind::Info => CALLOUT_BORDER_INFO,
    };
    format!(
        r#"<div style="background:#1c1c1c;border-left:3px solid {border};padding:14px 18px;margin:18px 0;color:#d2d2d2;font-size:14px;line-height:1.55">{body_html}</div>"#
    )
}

/// Two-column key/value table. Values are HTML-escaped — pass plain
/// strings (e.g., display names, ids, dates). Keys are also escaped
/// but typically literal copy.
pub fn section_kv(rows: &[(&str, &str)]) -> String {
    let mut out = format!(r#"<table role="presentation" cellpadding="0" cellspacing="0" border="0" style="{STYLE_KV}">"#);
    for (k, v) in rows {
        out.push_str(&format!(
            r#"<tr><td style="{STYLE_KV_K}">{}</td><td style="{STYLE_KV_V}">{}</td></tr>"#,
            html_escape(k),
            html_escape(v),
        ));
    }
    out.push_str("</table>");
    out
}

/// Inline pill — fits inside an eyebrow line for severity/category tags.
pub fn section_pip(kind: PipKind, label: &str) -> String {
    let (bg, fg) = match kind {
        PipKind::Default => (PIP_BG_DEFAULT, "#ffffff"),
        PipKind::Warn => (PIP_BG_WARN, "#000000"),
        PipKind::Danger => (PIP_BG_DANGER, "#ffffff"),
    };
    let safe = html_escape(label);
    format!(
        r#"<span style="display:inline-block;background:{bg};color:{fg};font-size:11px;text-transform:uppercase;letter-spacing:0.1em;padding:4px 9px;border-radius:99px;vertical-align:middle">{safe}</span>"#
    )
}

// ─── Date helpers ──────────────────────────────────────────────────────────

/// Format an epoch-ms timestamp in the server's local timezone using a
/// human-readable form: `"Sat, May 25 2026 at 8:09 PM"`. Self-hosted
/// product → operator's locale is the user's locale most of the time.
pub fn format_email_datetime(epoch_ms: i64) -> String {
    match Local.timestamp_millis_opt(epoch_ms).single() {
        Some(dt) => format_dt(dt),
        None => format!("epoch ms {epoch_ms}"),
    }
}

/// Same as [`format_email_datetime`] plus a relative tail —
/// `"Sat, May 25 2026 at 8:09 PM (in 7 days)"`. Returns the absolute-only
/// form when the delta is < 1 day either way (the absolute form already
/// reads well at that distance).
pub fn format_email_datetime_with_relative(epoch_ms: i64, now_ms: i64) -> String {
    let abs = format_email_datetime(epoch_ms);
    let delta_s = (epoch_ms - now_ms) / 1000;
    let suffix = match delta_s.abs() {
        d if d < 86_400 => return abs,
        d if d < 86_400 * 60 => {
            let days = d / 86_400;
            if delta_s > 0 {
                format!(" (in {days} day{})", if days == 1 { "" } else { "s" })
            } else {
                format!(" ({days} day{} ago)", if days == 1 { "" } else { "s" })
            }
        }
        _ => {
            // > 60d: just show the absolute date.
            return abs;
        }
    };
    format!("{abs}{suffix}")
}

fn format_dt(dt: DateTime<Local>) -> String {
    // chrono's strftime: `%a` = "Sat", `%b` = "May", `%-d` = day without
    // padding, `%Y` = year, `%-I` = 12h hour without padding, `%p` = AM/PM.
    dt.format("%a, %b %-d %Y at %-I:%M %p").to_string()
}

// ─── HTML escape ───────────────────────────────────────────────────────────
//
// Centralised so the eight templates can't drift on which entities they
// cover. We strip `<`, `>`, `&`, `"`, `'` — enough to defang
// attribute-injection and tag-injection inside our generated HTML.

pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escape_covers_attribute_injection_chars() {
        assert_eq!(
            html_escape("<a href=\"javascript:'pwn'\">x</a>"),
            "&lt;a href=&quot;javascript:&#x27;pwn&#x27;&quot;&gt;x&lt;/a&gt;",
        );
    }

    #[test]
    fn datetime_formats_human() {
        // 2026-05-25 20:09:00 UTC == 1779408540000 epoch ms.
        // We render in the server's local zone — assert only the bits
        // that don't depend on TZ so the test stays portable.
        let s = format_email_datetime(1_779_408_540_000);
        assert!(s.contains("2026"), "{s}");
        assert!(s.contains(":09"), "{s}");
    }

    #[test]
    fn relative_tail_is_omitted_for_subday_deltas() {
        let now = 1_779_408_540_000_i64;
        let then = now + 3_600_000; // +1h
        let s = format_email_datetime_with_relative(then, now);
        assert!(!s.contains(" ago"), "{s}");
        assert!(!s.contains("in"), "{s}");
    }

    #[test]
    fn relative_tail_pluralises_correctly() {
        let now = 1_700_000_000_000_i64;
        // +1 day exactly: should say "1 day", not "1 days".
        let s1 = format_email_datetime_with_relative(now + 86_400_000 + 1_000, now);
        assert!(s1.ends_with("(in 1 day)"), "{s1}");
        // +5 days: plural.
        let s5 = format_email_datetime_with_relative(now + 86_400_000 * 5 + 1_000, now);
        assert!(s5.ends_with("(in 5 days)"), "{s5}");
    }

    #[test]
    fn invalid_epoch_falls_back_gracefully() {
        // i64::MAX is well outside chrono's supported range — must not panic.
        let s = format_email_datetime(i64::MAX);
        assert!(s.starts_with("epoch ms "), "{s}");
    }
}
