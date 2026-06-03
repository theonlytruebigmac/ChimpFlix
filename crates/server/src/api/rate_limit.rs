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
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};
use serde_json::json;
use tokio::sync::RwLock;

use crate::client_ip::EffectiveClientIp;

pub type IpLimiter = RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock>;

/// Per-string limiter — used to throttle password resets per recipient
/// email address (independent of source IP). Defeats the "rotate
/// botnet IPs to email-bomb one victim" attack the audit flagged: a
/// distributed attacker can stay under the per-IP cap forever, but
/// they can't go above 3 resets per hour to any single inbox without
/// hitting this gate.
pub type StringLimiter = RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>;

/// Build a per-IP limiter for auth-style routes: 10 requests / minute
/// with a burst of 2. Tightened from burst=5 by WEEK 1 item #6 in
/// `docs/PUBLIC_RELEASE_HARDENING.md` — a 100-IP botnet got 500 free
/// password guesses per minute under the old config; this knocks
/// that down to 200, which combined with the lowered per-username
/// activation threshold makes a real difference. A human typing
/// their password wrong twice in a row still has 8 free attempts in
/// the next minute (well above any legitimate use).
pub fn auth_limiter() -> Arc<IpLimiter> {
    let quota =
        Quota::per_minute(NonZeroU32::new(10).unwrap()).allow_burst(NonZeroU32::new(2).unwrap());
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

/// Per-IP limiter for the Plex OAuth **poll** endpoint. This is a device-
/// flow poll a legitimate client hits roughly once a second while the user
/// authorizes the PIN, so it can't share the tight `auth_limiter` bucket
/// (that would 429 a normal login). 120/min with a burst of 20 sustains
/// ~1 poll/sec (refill is 2/sec) with headroom for a couple of concurrent
/// flows, while still capping an unauthenticated client from using poll as
/// an unbounded compute sink. PIN *creation* (`/auth/plex/start`) stays on
/// the strict `auth_limiter` since that's the cache-growth / Plex-quota
/// abuse vector.
pub fn plex_poll_limiter() -> Arc<IpLimiter> {
    let quota =
        Quota::per_minute(NonZeroU32::new(120).unwrap()).allow_burst(NonZeroU32::new(20).unwrap());
    Arc::new(RateLimiter::keyed(quota))
}

/// Per-email password-reset limiter: 3 requests / hour per address with
/// burst of 1. Tight on purpose — a legitimate human resets at most a
/// couple of times per hour even when troubleshooting; an attacker
/// trying to spam someone's inbox hits the wall on the second request.
pub fn reset_email_limiter() -> Arc<StringLimiter> {
    let quota =
        Quota::per_hour(NonZeroU32::new(3).unwrap()).allow_burst(NonZeroU32::new(1).unwrap());
    Arc::new(RateLimiter::keyed(quota))
}

/// Per-(user, item) limiter for `POST /items/{id}/report-issue`. Each
/// report fan-outs to every admin's email + creates one notification
/// row per admin, so unthrottled it's an amplification primitive. Cap
/// at 5 reports / hour for any given (user, item) pair. Different
/// items by the same user, or the same item by different users, share
/// nothing.
pub fn report_issue_limiter() -> Arc<StringLimiter> {
    let quota =
        Quota::per_hour(NonZeroU32::new(5).unwrap()).allow_burst(NonZeroU32::new(2).unwrap());
    Arc::new(RateLimiter::keyed(quota))
}

/// Middleware: rejects requests that exceed the per-IP quota. Falls back
/// to allowing the request through when no peer IP can be determined
/// (axum's ConnectInfo) — we never want a misconfigured proxy to take
/// the whole API offline.
///
/// The "client IP" here is the effective IP — proxy headers are only
/// honored when the immediate peer is in `trusted_proxies`. See
/// [`crate::client_ip`] for the resolution logic; the outer middleware
/// stashes the resolved IP into request extensions before this layer
/// runs.
pub async fn enforce(
    axum::extract::State(limiter): axum::extract::State<Arc<IpLimiter>>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = req
        .extensions()
        .get::<EffectiveClientIp>()
        .map(|e| e.0)
        .unwrap_or_else(|| peer.ip());
    if limiter.check_key(&ip).is_err() {
        return rate_limited_response("too many requests; try again shortly", 60);
    }
    next.run(req).await
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

/// Maximum number of distinct keys the tracker will hold at once.
/// Exceeding this limit triggers a sweep of expired entries before any
/// new key is admitted, and if the map is still full the new entry is
/// silently dropped (fail-open). This caps memory at roughly 100k * ~80B
/// ≈ 8 MB for the worst-case synthetic-username flood across many IPs.
const MAX_TRACKER_ENTRIES: usize = 100_000;

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
    /// Lockout policy: 3 failures → 60min, 6 → 6h, 10 → 24h. Tightened
    /// from the original 5/8/12 thresholds + 30s/5min/30min lockouts by
    /// WEEK 1 item #6 in `docs/PUBLIC_RELEASE_HARDENING.md`. The 60-min
    /// floor matches the "if you forgot your password, go fix it in
    /// /forgot-password instead of guessing more" UX expectation; the
    /// trailing 24h is a defense against persistent distributed
    /// brute-forcers across an extended window.
    pub async fn check(&self, key: &str) -> Option<Duration> {
        let guard = self.inner.read().await;
        let entry = guard.get(key)?;
        let until = entry.locked_until?;
        let now = Instant::now();
        if until > now { Some(until - now) } else { None }
    }

    pub async fn record_failure(&self, key: &str) {
        let mut guard = self.inner.write().await;
        // If we're at the size cap and this key isn't already tracked,
        // sweep expired entries first to reclaim space from old lockouts.
        // If still full after the sweep, drop the new entry (fail-open)
        // rather than growing the map unboundedly under a synthetic-username
        // flood across many IPs.
        if guard.len() >= MAX_TRACKER_ENTRIES && !guard.contains_key(key) {
            let now = Instant::now();
            guard.retain(|_, v| {
                v.locked_until.map_or(true, |until| until > now)
            });
            if guard.len() >= MAX_TRACKER_ENTRIES {
                return;
            }
        }
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
        0..=2 => Duration::from_secs(0),
        3..=5 => Duration::from_secs(60 * 60),
        6..=9 => Duration::from_secs(6 * 60 * 60),
        _ => Duration::from_secs(24 * 60 * 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lockout_progression() {
        let t = AttemptTracker::new();
        for _ in 0..2 {
            t.record_failure("alice").await;
        }
        assert!(t.check("alice").await.is_none()); // under threshold (≤ 2)
        t.record_failure("alice").await; // 3rd failure → 60min lockout
        let wait = t.check("alice").await.unwrap();
        assert!(wait.as_secs() > 30 * 60 && wait.as_secs() <= 60 * 60);
        t.record_success("alice").await;
        assert!(t.check("alice").await.is_none());
    }

    #[tokio::test]
    async fn long_lockout_escalates() {
        let t = AttemptTracker::new();
        for _ in 0..10 {
            t.record_failure("eve").await;
        }
        let wait = t.check("eve").await.unwrap();
        assert!(wait.as_secs() > 6 * 60 * 60); // 10th failure → 24h
    }
}
