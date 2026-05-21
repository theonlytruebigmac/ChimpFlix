//! Small CIDR-matching helpers shared by the auth bypass + remote-
//! stream policy code paths.
//!
//! Operators enter CIDR lists as comma-separated strings in admin
//! settings (matches Plex's UX); this module is the single place we
//! parse + match them so a malformed CIDR can't poison both call
//! sites.

use std::net::IpAddr;
use std::str::FromStr;

use ipnet::IpNet;

/// Parse a comma-separated CIDR list, skipping (with a warning trace)
/// any entries that don't round-trip. Empty input → empty Vec.
/// Bare IPs (no `/N` suffix) are accepted and treated as /32 or /128.
pub fn parse_cidr_list(raw: &str) -> Vec<IpNet> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| match parse_one(s) {
            Some(net) => Some(net),
            None => {
                tracing::warn!(entry = s, "ignoring malformed CIDR in settings list");
                None
            }
        })
        .collect()
}

/// Validate a comma-separated CIDR list without side effects. Returns
/// an error string naming the first bad entry; admin-settings
/// validation uses this so the operator can't save garbage.
pub fn validate_cidr_list(raw: &str) -> Result<(), String> {
    for entry in raw.split(',') {
        let s = entry.trim();
        if s.is_empty() {
            continue;
        }
        if parse_one(s).is_none() {
            return Err(format!("`{s}` is not a valid IP address or CIDR"));
        }
    }
    Ok(())
}

/// True when `ip` falls inside any of the CIDR ranges. Empty list
/// always returns false — operators clear the field to disable the
/// feature, never to match everything.
pub fn ip_in_list(ip: IpAddr, networks: &[IpNet]) -> bool {
    networks.iter().any(|n| n.contains(&ip))
}

fn parse_one(s: &str) -> Option<IpNet> {
    // ipnet::IpNet parses both `192.168.1.0/24` and `192.168.1.0` (the
    // latter as a /32). For a bare IP we want the same convenience —
    // try the IpNet parser first, fall back to IpAddr→/32 or /128.
    if let Ok(net) = IpNet::from_str(s) {
        return Some(net);
    }
    let ip = IpAddr::from_str(s).ok()?;
    Some(match ip {
        IpAddr::V4(_) => IpNet::from_str(&format!("{s}/32")).ok()?,
        IpAddr::V6(_) => IpNet::from_str(&format!("{s}/128")).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_skips_blanks_and_garbage() {
        let nets = parse_cidr_list("192.168.0.0/16, , 10.0.0.0/8,not-a-cidr,fe80::/10");
        assert_eq!(nets.len(), 3);
    }

    #[test]
    fn parse_list_accepts_bare_ips() {
        let nets = parse_cidr_list("192.168.1.5, 10.0.0.1");
        assert_eq!(nets.len(), 2);
        // Bare IPv4 should become /32.
        assert!(ip_in_list("192.168.1.5".parse().unwrap(), &nets));
        assert!(!ip_in_list("192.168.1.6".parse().unwrap(), &nets));
    }

    #[test]
    fn ip_in_list_handles_v4_and_v6() {
        let nets = parse_cidr_list("192.168.0.0/16,fe80::/10");
        assert!(ip_in_list("192.168.1.1".parse().unwrap(), &nets));
        assert!(ip_in_list("fe80::1".parse().unwrap(), &nets));
        assert!(!ip_in_list("8.8.8.8".parse().unwrap(), &nets));
        assert!(!ip_in_list("2001:db8::1".parse().unwrap(), &nets));
    }

    #[test]
    fn empty_list_matches_nothing() {
        let nets = parse_cidr_list("");
        assert!(!ip_in_list("127.0.0.1".parse().unwrap(), &nets));
        assert!(!ip_in_list("0.0.0.0".parse().unwrap(), &nets));
    }

    #[test]
    fn validate_returns_error_for_garbage() {
        assert!(validate_cidr_list("192.168.0.0/16").is_ok());
        assert!(validate_cidr_list("192.168.0.0/16, 10.0.0.0/8").is_ok());
        assert!(validate_cidr_list("").is_ok());
        let err = validate_cidr_list("192.168.0.0/16,oops").unwrap_err();
        assert!(err.contains("oops"));
    }
}
