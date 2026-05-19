//! Effective-client-IP resolution with trusted-proxy allowlist.
//!
//! The audit found that the old `header_client_ip` blindly trusted the
//! leftmost `X-Forwarded-For` entry — meaning any public caller could
//! send `X-Forwarded-For: 10.0.0.1` and (a) bypass per-IP rate limits,
//! (b) poison the IP recorded in audit_log and sessions, and (c) match
//! against `auth_bypass_cidrs` to become Owner without a cookie.
//!
//! Defense:
//!   1. The operator declares trusted proxies via `TRUSTED_PROXIES`
//!      (comma-separated CIDRs). Default: empty (= ignore all proxy
//!      headers; client IP = peer socket).
//!   2. Proxy headers are only honored when the immediate peer's
//!      `SocketAddr` is inside that allowlist.
//!   3. When walking `X-Forwarded-For`, we go right-to-left and stop at
//!      the first entry NOT in trusted_proxies — that's the originating
//!      client. Leftmost-wins (what we used to do) lets an attacker
//!      prefix `X-Forwarded-For` with whatever they want before Traefik
//!      appends the real peer.
//!   4. `CF-Connecting-IP` (single value, set by Cloudflare's edge) is
//!      preferred when peer is trusted — unambiguous, no chain to walk.
//!
//! Middleware stashes the resolved IP into request extensions as
//! [`EffectiveClientIp`]; handlers and the rate-limiter pull it from
//! there. Removing `rate_limit::header_client_ip` ensures the compiler
//! catches any callsite that still reaches for a raw header.

use std::net::{IpAddr, SocketAddr};

use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, Request};
use axum::middleware::Next;
use axum::response::Response;
use ipnet::IpNet;

use crate::net::ip_in_list;
use crate::state::AppState;

/// The client IP for this request after applying the trusted-proxy
/// allowlist. Stashed in request extensions by [`middleware`]; pulled
/// out via `Extension<EffectiveClientIp>` in handlers, or
/// `parts.extensions.get::<EffectiveClientIp>()` in custom extractors.
#[derive(Debug, Clone, Copy)]
pub struct EffectiveClientIp(pub IpAddr);

pub async fn middleware(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let ip = resolve(&state.trusted_proxies, peer.ip(), req.headers());
    req.extensions_mut().insert(EffectiveClientIp(ip));
    next.run(req).await
}

/// Compute the effective client IP. Visible for tests and for the
/// out-of-band callers (background tasks) that don't go through axum.
pub fn resolve(trusted: &[IpNet], peer: IpAddr, headers: &HeaderMap) -> IpAddr {
    if !ip_in_list(peer, trusted) {
        return peer;
    }
    if let Some(ip) = header_ip(headers, "cf-connecting-ip") {
        return ip;
    }
    if let Some(ip) = xff_rightmost_untrusted(headers, trusted) {
        return ip;
    }
    if let Some(ip) = header_ip(headers, "x-real-ip") {
        return ip;
    }
    peer
}

fn header_ip(headers: &HeaderMap, name: &'static str) -> Option<IpAddr> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
}

fn xff_rightmost_untrusted(headers: &HeaderMap, trusted: &[IpNet]) -> Option<IpAddr> {
    let raw = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())?;
    let mut last_trusted: Option<IpAddr> = None;
    for entry in raw.split(',').rev() {
        let trimmed = entry.trim();
        let Ok(ip) = trimmed.parse::<IpAddr>() else {
            continue;
        };
        if !ip_in_list(ip, trusted) {
            return Some(ip);
        }
        last_trusted = Some(ip);
    }
    // Every entry was a trusted proxy. Fall back to the leftmost one
    // we saw (= origin-facing proxy IP). Caller will likely use peer
    // when this returns None, but for completeness…
    last_trusted
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn nets(s: &str) -> Vec<IpNet> {
        crate::net::parse_cidr_list(s)
    }

    fn h(name: &'static str, value: &str) -> HeaderMap {
        let mut hm = HeaderMap::new();
        hm.insert(name, HeaderValue::from_str(value).unwrap());
        hm
    }

    #[test]
    fn no_trusted_proxies_returns_peer() {
        let peer: IpAddr = "203.0.113.5".parse().unwrap();
        let ip = resolve(&[], peer, &h("x-forwarded-for", "1.2.3.4"));
        assert_eq!(ip, peer);
    }

    #[test]
    fn untrusted_peer_ignores_proxy_headers() {
        let trusted = nets("172.16.0.0/12");
        let peer: IpAddr = "203.0.113.5".parse().unwrap();
        let ip = resolve(&trusted, peer, &h("x-forwarded-for", "1.2.3.4"));
        assert_eq!(ip, peer);
    }

    #[test]
    fn trusted_peer_honors_cf_connecting_ip() {
        let trusted = nets("172.16.0.0/12");
        let peer: IpAddr = "172.18.0.5".parse().unwrap();
        let ip = resolve(&trusted, peer, &h("cf-connecting-ip", "8.8.8.8"));
        assert_eq!(ip.to_string(), "8.8.8.8");
    }

    #[test]
    fn xff_returns_rightmost_untrusted_not_leftmost() {
        // Attacker-supplied: X-Forwarded-For: 10.0.0.1, REAL_CLIENT, TRAEFIK
        // The real client is at index N-1 (after Traefik stripped); the
        // attacker-supplied 10.0.0.1 is leftmost. Old logic returned
        // 10.0.0.1; new logic must return REAL_CLIENT.
        let trusted = nets("172.16.0.0/12");
        let peer: IpAddr = "172.18.0.5".parse().unwrap();
        let hm = h("x-forwarded-for", "10.0.0.1, 203.0.113.42, 172.18.0.2");
        let ip = resolve(&trusted, peer, &hm);
        assert_eq!(ip.to_string(), "203.0.113.42");
    }

    #[test]
    fn xff_falls_back_to_x_real_ip_when_xff_absent() {
        let trusted = nets("172.16.0.0/12");
        let peer: IpAddr = "172.18.0.5".parse().unwrap();
        let ip = resolve(&trusted, peer, &h("x-real-ip", "198.51.100.7"));
        assert_eq!(ip.to_string(), "198.51.100.7");
    }
}
