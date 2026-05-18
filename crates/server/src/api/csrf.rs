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
//!   1. If a request carries no auth cookie at all, skip the check —
//!      there's nothing to forge. (The handler will 401 by itself.)
//!   2. If `Origin` is present, it MUST match the server's `public_url`
//!      (when configured) OR appear in the configured `cors_origins`
//!      allow-list. Otherwise reject with 403.
//!   3. If `Origin` is absent but `Referer` is present, parse its
//!      scheme+host+port and apply the same check.
//!   4. If both are absent (Some HTTP clients omit both — curl by
//!      default, some mobile webviews), allow the request. CSRF
//!      requires a browser to be the abuser; clients that omit both
//!      headers are not browsers.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderName, Method, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::auth::COOKIE_NAME;
use crate::state::AppState;

pub async fn layer(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if !is_mutating(req.method()) {
        return next.run(req).await;
    }

    // No session cookie → nothing to forge. Let the handler 401 if it cares.
    let has_session = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| {
            c.split(';')
                .any(|p| p.trim().starts_with(&format!("{COOKIE_NAME}=")))
        });
    if !has_session {
        return next.run(req).await;
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

    // Both headers absent → not a browser. CSRF defense only matters for
    // browsers; allow through. The session cookie check still protects
    // authentication.
    let Some(candidate) = candidate else {
        return next.run(req).await;
    };

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

#[allow(dead_code)]
fn _csrf_header_name() -> HeaderName {
    // Reserved for a future double-submit cookie pattern if we add an
    // explicit CSRF token. Currently unused — origin validation is the
    // only defense and is sufficient given our SameSite=Lax cookie.
    HeaderName::from_static("x-csrf-token")
}
