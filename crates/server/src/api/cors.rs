//! Dynamic CORS middleware.
//!
//! `tower_http::cors::CorsLayer` is static — once built, its allow-list is
//! frozen. But our `cors_origins` setting is hot-reloaded behind the same
//! `SettingsCache` the rest of the server reads, so admin edits should take
//! effect without a restart. This middleware re-reads the allow-list on
//! every request.
//!
//! Behavior:
//! - Empty allow-list → no CORS headers added; same-origin requests work
//!   normally and cross-origin browsers will block the response themselves.
//! - Allow-list contains the request's `Origin` (or `*`) → echo the
//!   origin back with `Access-Control-Allow-Credentials: true`. We never
//!   send the literal `*` because the spec forbids combining it with
//!   credentials, and we rely on cookie auth.
//! - Preflight `OPTIONS` requests short-circuit here with the appropriate
//!   `Access-Control-Allow-*` headers; they never reach the inner router.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::state::AppState;

const PREFLIGHT_MAX_AGE_SECS: &str = "600";
const DEFAULT_ALLOWED_HEADERS: &str = "content-type, authorization, x-requested-with";
const DEFAULT_ALLOWED_METHODS: &str = "GET, POST, PATCH, PUT, DELETE, OPTIONS";

pub async fn layer(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let is_preflight = req.method() == Method::OPTIONS
        && req
            .headers()
            .contains_key(HeaderName::from_static("access-control-request-method"));

    let allowed_origin = match origin.as_deref() {
        Some(o) if origin_allowed(&state, o).await => Some(o.to_owned()),
        _ => None,
    };

    if is_preflight {
        let mut resp = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .expect("build preflight response");
        if let Some(o) = allowed_origin.as_deref() {
            write_cors_headers(resp.headers_mut(), o, true);
        }
        return resp;
    }

    let mut resp = next.run(req).await;
    if let Some(o) = allowed_origin.as_deref() {
        write_cors_headers(resp.headers_mut(), o, false);
    }
    resp
}

async fn origin_allowed(state: &AppState, origin: &str) -> bool {
    let s = state.settings.read().await;
    let allow_list: Vec<String> = serde_json::from_str(&s.cors_origins).unwrap_or_default();
    if allow_list.is_empty() {
        return false;
    }
    // SECURITY: never honor `*` in the allow-list. With
    // `Access-Control-Allow-Credentials: true` (which we always send),
    // echoing an arbitrary origin produces the classic "credentialled
    // wildcard" misconfig that lets any external site read auth'd
    // responses from our API. Operators who genuinely need an open
    // API would have to remove the credentialled-CORS coupling first,
    // which is a deliberate design choice we don't want one stray
    // settings entry to undo.
    allow_list.iter().any(|entry| {
        let e = entry.trim();
        !e.is_empty() && e != "*" && e.eq_ignore_ascii_case(origin)
    })
}

fn write_cors_headers(headers: &mut HeaderMap, origin: &str, is_preflight: bool) {
    if let Ok(v) = HeaderValue::from_str(origin) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));

    if is_preflight {
        // Fixed allow-lists rather than echoing the preflight request.
        // Echoing `Access-Control-Request-Headers` lets a forged
        // preflight bypass any custom-header CSRF defence we might
        // add later (`X-CSRF-Token` would trivially appear in any
        // future `access-control-request-headers: x-csrf-token`).
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static(DEFAULT_ALLOWED_METHODS),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static(DEFAULT_ALLOWED_HEADERS),
        );
        headers.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static(PREFLIGHT_MAX_AGE_SECS),
        );
    }
}
