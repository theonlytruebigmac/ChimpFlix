//! SSRF defense for outbound HTTP requests the operator can target.
//!
//! Webhook URLs, the network reachability test, and any future
//! operator-supplied URL flow through here. Without this guard, a
//! compromised owner session (or an owner pasting an attacker-supplied
//! URL into webhooks) could:
//!   * Hit cloud metadata services (`http://169.254.169.254/...`) and
//!     exfiltrate IAM credentials.
//!   * Probe internal services on `127.0.0.1` / `10.0.0.0/8` /
//!     `172.16.0.0/12` / `192.168.0.0/16` / Docker bridge ranges that
//!     aren't reachable from the public internet.
//!   * Pivot through the server to RFC 1918 hosts on the operator's LAN.
//!
//! Strategy: resolve the URL's hostname, walk every resolved IP, and
//! reject if any falls into a blocked range. This accepts a small TOCTOU
//! window with reqwest's own resolver (DNS rebinding could in principle
//! return a different IP at request time), but the deliberate operator
//! flows that ship today don't observe response bodies under attacker
//! control, so the practical residual risk is bounded.

use std::net::{IpAddr, Ipv4Addr};

use reqwest::Url;
use tokio::net::lookup_host;

/// Validate that `raw` is safe to fetch as an outbound HTTP/HTTPS URL.
/// Returns the parsed `Url` on success; a human-readable reason on
/// failure (suitable for surfacing in admin validation errors).
pub async fn ensure_safe_outbound_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|e| format!("invalid url: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        s => return Err(format!("scheme `{s}` is not allowed; use http or https")),
    }
    let host = url
        .host_str()
        .ok_or_else(|| "url has no host".to_string())?
        .to_string();
    let port = url
        .port_or_known_default()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });

    // Literal IP in the host position — verify directly without DNS.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_unsafe_ip(ip) {
            return Err(blocked_msg(&host, ip));
        }
        return Ok(url);
    }

    // DNS resolve. `lookup_host` returns every address record (A + AAAA);
    // we reject if any is unsafe so an attacker can't "race" the
    // resolver into picking a safe answer.
    let addrs = lookup_host(format!("{host}:{port}"))
        .await
        .map_err(|e| format!("dns lookup failed for `{host}`: {e}"))?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return Err(format!("host `{host}` did not resolve to any address"));
    }
    for sock in addrs {
        let ip = sock.ip();
        if is_unsafe_ip(ip) {
            return Err(blocked_msg(&host, ip));
        }
    }
    Ok(url)
}

fn blocked_msg(host: &str, ip: IpAddr) -> String {
    format!(
        "host `{host}` resolves to a blocked address ({ip}) — outbound webhook / \
         reachability requests cannot target loopback, link-local, private \
         (RFC 1918 / CGNAT), multicast, broadcast, unspecified, or cloud-metadata \
         (169.254.169.254) addresses"
    )
}

/// True when this IP must not be the target of an operator-controlled
/// outbound HTTP request. The list is the conventional SSRF blocklist
/// plus the cloud-metadata endpoint.
fn is_unsafe_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_unsafe_v4(v4),
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_multicast() || v6.is_unspecified() {
                return true;
            }
            // Link-local (fe80::/10).
            if (v6.segments()[0] & 0xFFC0) == 0xFE80 {
                return true;
            }
            // Unique-local addresses (fc00::/7).
            if (v6.segments()[0] & 0xFE00) == 0xFC00 {
                return true;
            }
            // Mapped IPv4 — extract and re-check.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_unsafe_v4(v4);
            }
            false
        }
    }
}

fn is_unsafe_v4(v4: Ipv4Addr) -> bool {
    if v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_broadcast()
        || v4.is_unspecified()
    {
        return true;
    }
    // Cloud metadata services (AWS, Azure, GCP, Hetzner, …) all sit at
    // 169.254.169.254. `is_link_local` already covers this, but list
    // explicitly so the intent is searchable.
    if v4 == Ipv4Addr::new(169, 254, 169, 254) {
        return true;
    }
    // Carrier-grade NAT (RFC 6598): 100.64.0.0/10. Not "private" per
    // stdlib but conventionally not exposed to public clients.
    let octets = v4.octets();
    if octets[0] == 100 && (octets[1] & 0xC0) == 0x40 {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_loopback_literal() {
        let e = ensure_safe_outbound_url("http://127.0.0.1/")
            .await
            .unwrap_err();
        assert!(e.contains("blocked"), "{e}");
    }

    #[tokio::test]
    async fn rejects_metadata_service() {
        let e = ensure_safe_outbound_url("http://169.254.169.254/latest/meta-data/")
            .await
            .unwrap_err();
        assert!(e.contains("blocked"), "{e}");
    }

    #[tokio::test]
    async fn rejects_private_literal() {
        let e = ensure_safe_outbound_url("http://10.0.0.1/")
            .await
            .unwrap_err();
        assert!(e.contains("blocked"), "{e}");
    }

    #[tokio::test]
    async fn rejects_link_local_v6() {
        let e = ensure_safe_outbound_url("http://[fe80::1]/")
            .await
            .unwrap_err();
        assert!(e.contains("blocked"), "{e}");
    }

    #[tokio::test]
    async fn rejects_non_http_scheme() {
        let e = ensure_safe_outbound_url("file:///etc/passwd")
            .await
            .unwrap_err();
        assert!(e.contains("scheme"), "{e}");
    }

    #[test]
    fn unsafe_v4_blocks_expected_ranges() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
        ] {
            let v4: Ipv4Addr = ip.parse().unwrap();
            assert!(is_unsafe_v4(v4), "{ip} should be blocked");
        }
    }

    #[test]
    fn unsafe_v4_allows_public() {
        for ip in ["8.8.8.8", "1.1.1.1", "203.0.113.5"] {
            let v4: Ipv4Addr = ip.parse().unwrap();
            assert!(!is_unsafe_v4(v4), "{ip} should be allowed");
        }
    }
}
