//! In-memory rate limiting for authentication and other sensitive routes.
//!
//! Two layers compose:
//!   * Per-IP token bucket (governor) — broad shield against scraping
//!     and credential-stuffing from a single source.
//!   * Per-key lockout counter (this module) — narrow, tracks failed
//!     attempts against a specific identity (e.g. a username). Used by
//!     the login handler to apply progressive backoff and temporary
//!     lockouts on repeated failures from anywhere.
//!
//! Storage is process-local. A single-instance self-hosted deployment is
//! the assumed shape; horizontal scaling would need to lift this into a
//! shared store, but the API would stay the same.

use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderName, HeaderValue, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};
use serde_json::json;
use tokio::sync::RwLock;

pub type IpLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

/// Build a per-IP limiter for auth-style routes: 10 requests / minute
/// with a burst of 5. Tight enough to stop a brute-force, loose enough
/// that a human typing wrong a few times doesn't get locked.
pub fn auth_limiter() -> Arc<IpLimiter> {
    let quota =
        Quota::per_minute(NonZeroU32::new(10).unwrap()).allow_burst(NonZeroU32::new(5).unwrap());
    Arc::new(RateLimiter::keyed(quota))
}

/// Slightly looser limiter for endpoints that admins hit interactively
/// (invite issuance, reset triggers). 30/min with burst of 10.
#[allow(dead_code)] // wired by Phase 2 (invite + password-reset routes)
pub fn admin_limiter() -> Arc<IpLimiter> {
    let quota =
        Quota::per_minute(NonZeroU32::new(30).unwrap()).allow_burst(NonZeroU32::new(10).unwrap());
    Arc::new(RateLimiter::keyed(quota))
}

/// Middleware: rejects requests that exceed the per-IP quota. Falls back
/// to allowing the request through when no peer IP can be determined
/// (axum's ConnectInfo) — we never want a misconfigured proxy to take
/// the whole API offline.
pub async fn enforce(
    axum::extract::State(limiter): axum::extract::State<Arc<IpLimiter>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = client_ip(&req, peer.ip());
    if limiter.check_key(&ip).is_err() {
        return rate_limited_response("too many requests; try again shortly", 60);
    }
    next.run(req).await
}

/// Pull the client IP, honoring `X-Forwarded-For` when present. We only
/// trust the leftmost entry — `X-Forwarded-For` is a comma-separated
/// chain and the leftmost value is the original client. Operators behind
/// untrusted proxies should strip this header upstream.
/// Extract the client IP from the request headers — public so login
/// handlers can use the same logic when recording last-login-from-IP.
pub fn header_client_ip(headers: &axum::http::HeaderMap) -> Option<String> {
    if let Some(value) = headers
        .get(HeaderName::from_static("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
    {
        if let Some(first) = value.split(',').next() {
            let trimmed = first.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    headers
        .get(HeaderName::from_static("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn client_ip<B>(req: &Request<B>, fallback: IpAddr) -> IpAddr {
    if let Some(value) = req
        .headers()
        .get(HeaderName::from_static("x-forwarded-for"))
        .and_then(|v| v.to_str().ok())
    {
        if let Some(first) = value.split(',').next() {
            if let Ok(parsed) = first.trim().parse::<IpAddr>() {
                return parsed;
            }
        }
    }
    if let Some(value) = req
        .headers()
        .get(HeaderName::from_static("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
    {
        return value;
    }
    fallback
}

fn rate_limited_response(message: &str, retry_after_s: u64) -> Response {
    let body = axum::Json(json!({
        "error": {
            "code": "too_many_requests",
            "message": message,
        }
    }));
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, body).into_response();
    if let Ok(v) = HeaderValue::from_str(&retry_after_s.to_string()) {
        resp.headers_mut().insert(header::RETRY_AFTER, v);
    }
    resp
}

// ---------------------------------------------------------------------------
// Per-identity attempt tracker (for login lockouts).
// ---------------------------------------------------------------------------

/// Tracks failed attempts against a specific key (typically a username
/// normalized to lowercase). Used by the login handler — NOT exposed
/// as a middleware because the handler needs to record success/failure
/// after the password check completes.
#[derive(Clone, Default)]
pub struct AttemptTracker {
    inner: Arc<RwLock<HashMap<String, AttemptState>>>,
}

#[derive(Clone, Copy)]
struct AttemptState {
    failures: u32,
    locked_until: Option<Instant>,
}

impl AttemptTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the duration the caller must wait, or None if it can proceed.
    /// Lockout policy: 5 failures → 30s, 8 → 5min, 12 → 30min.
    pub async fn check(&self, key: &str) -> Option<Duration> {
        let guard = self.inner.read().await;
        let entry = guard.get(key)?;
        let until = entry.locked_until?;
        let now = Instant::now();
        if until > now {
            Some(until - now)
        } else {
            None
        }
    }

    pub async fn record_failure(&self, key: &str) {
        let mut guard = self.inner.write().await;
        let entry = guard.entry(key.to_owned()).or_insert(AttemptState {
            failures: 0,
            locked_until: None,
        });
        entry.failures = entry.failures.saturating_add(1);
        entry.locked_until = Some(Instant::now() + backoff_for(entry.failures));
    }

    pub async fn record_success(&self, key: &str) {
        let mut guard = self.inner.write().await;
        guard.remove(key);
    }

    /// Admin-initiated unlock. Wipes the tracker entry for the given
    /// key so the user can try again immediately.
    pub async fn clear(&self, key: &str) {
        let mut guard = self.inner.write().await;
        guard.remove(key);
    }
}

fn backoff_for(failures: u32) -> Duration {
    match failures {
        0..=4 => Duration::from_secs(0),
        5..=7 => Duration::from_secs(30),
        8..=11 => Duration::from_secs(5 * 60),
        _ => Duration::from_secs(30 * 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lockout_progression() {
        let t = AttemptTracker::new();
        for _ in 0..4 {
            t.record_failure("alice").await;
        }
        assert!(t.check("alice").await.is_none()); // under threshold
        t.record_failure("alice").await; // 5th failure
        let wait = t.check("alice").await.unwrap();
        assert!(wait.as_secs() <= 30 && wait.as_secs() > 0);
        t.record_success("alice").await;
        assert!(t.check("alice").await.is_none());
    }
}
