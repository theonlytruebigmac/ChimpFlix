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

pub async fn layer(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
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
        let req_headers = req.headers().clone();
        let mut resp = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .expect("build preflight response");
        if let Some(o) = allowed_origin.as_deref() {
            write_cors_headers(resp.headers_mut(), o, Some(&req_headers));
        }
        return resp;
    }

    let mut resp = next.run(req).await;
    if let Some(o) = allowed_origin.as_deref() {
        write_cors_headers(resp.headers_mut(), o, None);
    }
    resp
}

async fn origin_allowed(state: &AppState, origin: &str) -> bool {
    let s = state.settings.read().await;
    let allow_list: Vec<String> = serde_json::from_str(&s.cors_origins).unwrap_or_default();
    if allow_list.is_empty() {
        return false;
    }
    allow_list.iter().any(|entry| {
        let e = entry.trim();
        e == "*" || e.eq_ignore_ascii_case(origin)
    })
}

fn write_cors_headers(
    headers: &mut HeaderMap,
    origin: &str,
    preflight_req_headers: Option<&HeaderMap>,
) {
    if let Ok(v) = HeaderValue::from_str(origin) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));

    if let Some(req_headers) = preflight_req_headers {
        let methods_value = req_headers
            .get(HeaderName::from_static("access-control-request-method"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or(DEFAULT_ALLOWED_METHODS);
        if let Ok(v) = HeaderValue::from_str(methods_value) {
            headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, v);
        }
        let headers_value = req_headers
            .get(HeaderName::from_static("access-control-request-headers"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or(DEFAULT_ALLOWED_HEADERS);
        if let Ok(v) = HeaderValue::from_str(headers_value) {
            headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, v);
        }
        headers.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static(PREFLIGHT_MAX_AGE_SECS),
        );
    }
}
