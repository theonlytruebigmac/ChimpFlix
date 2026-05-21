//! CSRF defense via Origin / Referer validation.
//!
//! Our cookies are SameSite=Lax which already blocks the bulk of cross-site
//! attacks (browsers won't send the cookie on cross-site POST/PUT/PATCH/
//! DELETE that are top-level navigations). This middleware adds a second
//! layer for any cross-site requests that the SameSite policy might still
//! let through (notably, `<form method="GET">` attacks aren't a thing
//! because GET isn't supposed to mutate state, and we never expose a
//! mutating GET).
//!
//! Policy (mutating methods only — POST/PUT/PATCH/DELETE):
//!   1. **Auth endpoints** (`/auth/login`, `/auth/setup`, `/auth/register`,
//!      `/auth/password-reset/*`, `/auth/2fa/login`) ALWAYS require a
//!      same-origin Origin/Referer — these are unauthenticated by design
//!      (no session cookie yet), and the audit flagged that the "no
//!      cookie = skip" shortcut lets a malicious site log a victim into
//!      an attacker-controlled account ("login CSRF").
//!   2. For all other mutating routes:
//!        - If no session cookie is present, skip (nothing to forge).
//!        - If `Origin` is present, it MUST match the server's
//!          `public_url` OR appear in `cors_origins`.
//!        - If `Origin` is absent but `Referer` is present, same check
//!          against parsed scheme+host+port.
//!        - If both are absent (curl, some mobile webviews), allow —
//!          a session cookie that arrived without an Origin/Referer is
//!          almost certainly a non-browser client.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::auth::{CSRF_HEADER_NAME, cookie_name, csrf_cookie_name};
use crate::state::AppState;
use chimpflix_library::queries;

/// Path prefixes / suffixes of mutating routes that must NEVER bypass
/// CSRF — even when no session cookie is present. These are the
/// session-establishing endpoints; a successful CSRF here logs the
/// victim into the attacker's account or triggers a password reset on
/// the attacker's chosen address.
const STRICT_CSRF_PATHS: &[&str] = &[
    "/api/v1/auth/login",
    "/api/v1/auth/setup",
    "/api/v1/auth/register",
    "/api/v1/auth/password-reset/request",
    "/api/v1/auth/password-reset/confirm",
    "/api/v1/auth/2fa/login",
    "/api/v1/auth/me/email/request-change",
    "/api/v1/auth/me/email/confirm",
];

/// Path suffixes that skip the double-submit CSRF *token* check (but
/// keep Origin/Referer validation). Reserved for endpoints that must be
/// callable via `navigator.sendBeacon`, which the browser doesn't let
/// us attach custom headers to — including `X-CSRF-Token`. These
/// endpoints are still protected by SameSite=Lax cookies + Origin
/// matching, and they're idempotent and scoped to the caller's own
/// session (the handler enforces `session.user_id == user.id`), so the
/// remaining attack surface is "an attacker can ping their own session
/// closed", which is harmless.
///
/// The `/close` endpoint is the canonical case: the player uses
/// sendBeacon on pagehide / SPA teardown to guarantee delivery during
/// unload, and without this exemption every teardown returned 403,
/// leaving sessions running until the idle reaper swept them — which
/// also meant `find_compatible` adopted them on the next playback
/// request, breaking backward seeks that should have rolled a fresh
/// session.
fn allows_beacon_csrf_skip(path: &str) -> bool {
    // Match `/api/v1/stream/sessions/{id}/close` exactly. We need to
    // be strict here because a future route ending in `/close` would
    // otherwise accidentally inherit this exemption.
    //
    //   * `strip_prefix` peels off the route group.
    //   * `strip_suffix` ensures the path *ends* with `/close`.
    //   * We then verify the middle is a SINGLE non-empty segment —
    //     enforced by splitting and counting, not just "no slashes".
    //     The split-count is robust to edge cases like consecutive
    //     slashes (`//`, `/x//`) or trailing slashes that bare
    //     `contains` would miss.
    let Some(rest) = path.strip_prefix("/api/v1/stream/sessions/") else {
        return false;
    };
    let Some(after_id) = rest.strip_suffix("/close") else {
        return false;
    };
    let segments: Vec<&str> = after_id.split('/').collect();
    segments.len() == 1 && !segments[0].is_empty()
}

#[cfg(test)]
mod beacon_skip_tests {
    use super::allows_beacon_csrf_skip;

    #[test]
    fn accepts_canonical_close() {
        assert!(allows_beacon_csrf_skip(
            "/api/v1/stream/sessions/abc123/close"
        ));
    }
    #[test]
    fn rejects_no_session_id() {
        assert!(!allows_beacon_csrf_skip("/api/v1/stream/sessions//close"));
    }
    #[test]
    fn rejects_extra_suffix() {
        assert!(!allows_beacon_csrf_skip(
            "/api/v1/stream/sessions/abc/close/extra"
        ));
    }
    #[test]
    fn rejects_double_slash_in_id() {
        assert!(!allows_beacon_csrf_skip(
            "/api/v1/stream/sessions/abc//close"
        ));
    }
    #[test]
    fn rejects_unrelated_routes() {
        assert!(!allows_beacon_csrf_skip("/api/v1/auth/login"));
        assert!(!allows_beacon_csrf_skip("/api/v1/stream/sessions/abc"));
    }
}

pub async fn layer(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if !is_mutating(req.method()) {
        return next.run(req).await;
    }

    let path = req.uri().path();
    let strict = STRICT_CSRF_PATHS.iter().any(|p| path == *p);

    // Non-strict route + no session cookie → nothing to forge. Let the
    // handler 401 if it cares. Strict routes always run the Origin check
    // regardless of cookie presence — they're the entry to a session.
    let session_name = cookie_name(state.auth.cookie_secure);
    let cookie_header_str = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());
    if !strict {
        let has_session = cookie_header_str.as_deref().is_some_and(|c| {
            c.split(';')
                .any(|p| p.trim().starts_with(&format!("{session_name}=")))
        });
        if !has_session {
            return next.run(req).await;
        }
    }

    // Double-submit CSRF token check. Skip for strict routes (they
    // don't have a session yet, so no companion cookie exists). For
    // every other mutating request: the `cf_csrf` cookie value must
    // be echoed in `X-CSRF-Token`, and the two must match exactly.
    // This is the second layer behind SameSite=Lax + Origin/Referer
    // — it defends against (a) Origin-stripping clients, (b) future
    // same-origin XSS bugs in the frontend that haven't yet stolen
    // the session cookie (HttpOnly) but could still issue fetch
    // requests, and (c) any header-injection that lands on the
    // mutating-route surface.
    //
    // Also skip for the narrow set of sendBeacon-only endpoints
    // (currently `/stream/sessions/{id}/close`) where the browser
    // doesn't permit custom headers. The Origin check below still
    // runs for these.
    let beacon_skip = allows_beacon_csrf_skip(path);
    if !strict && !beacon_skip {
        let csrf_name = csrf_cookie_name(state.auth.cookie_secure);
        let cookie_value = cookie_header_str
            .as_deref()
            .and_then(|c| find_named_cookie(c, csrf_name));
        let header_value = req
            .headers()
            .get(CSRF_HEADER_NAME)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        match (cookie_value, header_value) {
            (Some(c), Some(h)) if !c.is_empty() && c == h => {}
            _ => return reject_csrf().into_response(),
        }
    }

    let origin_value = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());
    let referer_value = req
        .headers()
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| origin_from_url(s));

    let candidate = origin_value.or(referer_value);

    let Some(candidate) = candidate else {
        // Strict routes refuse the "no headers" escape — a real browser
        // either sends Origin or Referer. Treat the absence as a CSRF
        // attempt for login/register/reset, and let it through for
        // already-authenticated routes (where SameSite cookies and the
        // session check do the heavy lifting and some clients legitimately
        // omit these headers).
        if strict {
            return reject().into_response();
        }
        return next.run(req).await;
    };

    // Setup-mode escape hatch: when the server has no owner account
    // yet, `public_url` is NULL and `cors_origins` is `[]`, so the
    // origin check below rejects every request — including the very
    // setup POST that's supposed to bootstrap the system. Letting the
    // setup endpoint through (Origin/Referer still required, just
    // not value-validated) breaks that chicken-and-egg. The setup
    // handler then persists the request's origin as `public_url` so
    // subsequent strict routes pass without manual config.
    //
    // Limited to `/api/v1/auth/setup` and only while
    // `is_in_setup_mode()` is true; the moment the owner account is
    // created the bypass closes for everyone. The remaining attack
    // surface — a third party reaching an un-setup server before the
    // operator does — is the same surface as exposing any un-setup
    // admin tool to the public network.
    if path == "/api/v1/auth/setup" {
        let in_setup = queries::is_in_setup_mode(&state.pool)
            .await
            .unwrap_or(false);
        if in_setup {
            return next.run(req).await;
        }
    }

    if !origin_permitted(&state, &candidate).await {
        return reject().into_response();
    }

    next.run(req).await
}

fn is_mutating(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// Strip path/query from a Referer URL, returning just `scheme://host[:port]`.
fn origin_from_url(s: &str) -> Option<String> {
    let after_scheme = s.find("://")?;
    let rest = &s[after_scheme + 3..];
    let host_end = rest.find('/').unwrap_or(rest.len());
    Some(format!("{}{}", &s[..after_scheme + 3], &rest[..host_end]))
}

async fn origin_permitted(state: &AppState, candidate: &str) -> bool {
    let s = state.settings.read().await;
    if let Some(public_url) = s.public_url.as_deref() {
        let public_origin = origin_from_url(public_url).unwrap_or_else(|| public_url.to_string());
        if candidate.eq_ignore_ascii_case(&public_origin) {
            return true;
        }
    }
    let allow_list: Vec<String> = serde_json::from_str(&s.cors_origins).unwrap_or_default();
    allow_list
        .iter()
        .any(|entry| entry.trim().eq_ignore_ascii_case(candidate))
}

fn reject() -> impl IntoResponse {
    let body = axum::Json(json!({
        "error": { "code": "csrf_rejected", "message": "request origin not permitted" }
    }));
    (StatusCode::FORBIDDEN, body)
}

fn reject_csrf() -> impl IntoResponse {
    let body = axum::Json(json!({
        "error": {
            "code": "csrf_token_missing",
            "message": "request missing or mismatched CSRF token; reload the page"
        }
    }));
    (StatusCode::FORBIDDEN, body)
}

/// Look up `name` in a `Cookie:` header value. Returns `Some(value)` on
/// hit, `None` otherwise. Intentionally narrow — we don't want a
/// general cookie parser; just enough to read our companion token.
fn find_named_cookie(header_value: &str, name: &str) -> Option<String> {
    for chunk in header_value.split(';') {
        let trimmed = chunk.trim();
        if let Some(rest) = trimmed.strip_prefix(&format!("{name}=")) {
            return Some(rest.to_string());
        }
    }
    None
}
