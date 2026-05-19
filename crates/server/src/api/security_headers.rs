//! Security headers middleware.
//!
//! Adds defense-in-depth response headers on every API reply. The Next.js
//! frontend ships its own header set in `next.config.ts` for HTML routes;
//! these are scoped to JSON/API responses but applied uniformly so any
//! direct API consumer (curl, third-party clients) also gets the same
//! guarantees.
//!
//! The CSP we send is intentionally restrictive — the API only ever
//! returns JSON / opaque binary payloads, so we declare `default-src
//! 'none'` and explicitly allow `frame-ancestors 'none'` to refuse
//! framing. HTML pages have their own CSP wired in next.config.ts.

use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, Request, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::state::AppState;

const HSTS_VALUE: &str = "max-age=31536000; includeSubDomains";
const REFERRER_POLICY: &str = "strict-origin-when-cross-origin";
const PERMISSIONS_POLICY: &str = "accelerometer=(), browsing-topics=(), camera=(), \
                                  display-capture=(), geolocation=(), gyroscope=(), \
                                  interest-cohort=(), magnetometer=(), microphone=(), \
                                  payment=(), serial=(), usb=(), xr-spatial-tracking=()";
const API_CSP: &str = "default-src 'none'; frame-ancestors 'none'";
const X_CONTENT_TYPE: &str = "nosniff";
const X_FRAME_OPTIONS: &str = "DENY";
const CROSS_ORIGIN_OPENER: &str = "same-origin";
const CROSS_ORIGIN_RESOURCE: &str = "same-origin";
/// API responses are JSON / opaque bytes — no third-party iframes, no
/// cross-origin scripts. `require-corp` pairs with COOP=same-origin to
/// enable cross-origin isolation, which protects against Spectre-class
/// side-channels and is a precondition for high-resolution timers in
/// any future frontend feature.
const CROSS_ORIGIN_EMBEDDER: &str = "require-corp";
/// Disable the legacy XSS auditor. Modern guidance is that `1; mode=block`
/// introduced known XSS vulnerabilities; `0` is the safe value. (Some
/// upstream proxies still inject `1; mode=block`; we send `0` explicitly
/// so it sticks.)
const X_XSS_PROTECTION: &str = "0";

pub async fn layer(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();

    set_if_absent(
        headers,
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static(X_CONTENT_TYPE),
    );
    set_if_absent(
        headers,
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static(X_FRAME_OPTIONS),
    );
    set_if_absent(
        headers,
        header::REFERRER_POLICY,
        HeaderValue::from_static(REFERRER_POLICY),
    );
    set_if_absent(
        headers,
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    set_if_absent(
        headers,
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(API_CSP),
    );
    set_if_absent(
        headers,
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static(CROSS_ORIGIN_OPENER),
    );
    set_if_absent(
        headers,
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static(CROSS_ORIGIN_RESOURCE),
    );
    set_if_absent(
        headers,
        HeaderName::from_static("cross-origin-embedder-policy"),
        HeaderValue::from_static(CROSS_ORIGIN_EMBEDDER),
    );
    // Always send X-XSS-Protection: 0 — Traefik / Cloudflare layers in
    // many setups inject `1; mode=block` by default, which is unsafe.
    // We overwrite with the modern-recommended value to be sure.
    headers.insert(
        HeaderName::from_static("x-xss-protection"),
        HeaderValue::from_static(X_XSS_PROTECTION),
    );

    // HSTS only when we're sure the deployment is HTTPS — see auth/mod.rs
    // for how cookie_secure gets derived from APP_PUBLIC_ORIGIN. Sending
    // HSTS over plain HTTP would either be ignored (best case) or pin
    // a misconfigured origin (worst case).
    if state.auth.cookie_secure {
        set_if_absent(
            headers,
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static(HSTS_VALUE),
        );
    }

    resp
}

fn set_if_absent(
    headers: &mut axum::http::HeaderMap,
    name: HeaderName,
    value: HeaderValue,
) {
    if !headers.contains_key(&name) {
        headers.insert(name, value);
    }
}
