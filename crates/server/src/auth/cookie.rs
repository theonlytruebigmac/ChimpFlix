//! Signed session cookie format.
//!
//! The cookie value is `{session_id}:{nonce_hex}:{hmac_hex}` where the
//! HMAC is `HMAC-SHA256({session_id}:{nonce_hex}, secret)`. The HMAC ties
//! the cookie to the server's secret; the nonce ties it to a specific
//! session row (revoking the row revokes the cookie).

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn build_value(session_id: i64, nonce: &[u8; 32], secret: &[u8]) -> String {
    let payload = format!("{session_id}:{}", hex::encode(nonce));
    let sig = sign(&payload, secret);
    format!("{payload}:{}", hex::encode(sig))
}

pub fn parse_value(value: &str, secret: &[u8]) -> Option<(i64, [u8; 32])> {
    let mut parts = value.splitn(3, ':');
    let session_id_s = parts.next()?;
    let nonce_hex = parts.next()?;
    let sig_hex = parts.next()?;

    let session_id: i64 = session_id_s.parse().ok()?;
    let nonce_bytes = hex::decode(nonce_hex).ok()?;
    if nonce_bytes.len() != 32 {
        return None;
    }
    let sig = hex::decode(sig_hex).ok()?;

    let payload = format!("{session_id}:{nonce_hex}");
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(payload.as_bytes());
    mac.verify_slice(&sig).ok()?;

    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&nonce_bytes);
    Some((session_id, nonce))
}

fn sign(payload: &str, secret: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Build a Set-Cookie header value for a new session. The cookie name
/// is `__Host-cf_session` over HTTPS (forces Secure + Path=/ at the
/// browser level) and `cf_session` over plain HTTP.
pub fn set_cookie_header(value: &str, max_age_s: i64, secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_s}{secure_attr}",
        name = crate::auth::cookie_name(secure),
    )
}

/// Build a Set-Cookie header value that clears the session cookie.
pub fn clear_cookie_header(secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!(
        "{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure_attr}",
        name = crate::auth::cookie_name(secure),
    )
}

/// Derive the double-submit CSRF token for a given (session_id, nonce)
/// tuple. Deterministic: same session always produces the same token,
/// so we don't need to store anything server-side. Different secret =
/// different token, so a session_id leak alone doesn't let an attacker
/// mint a CSRF cookie.
pub fn csrf_token(session_id: i64, nonce: &[u8; 32], secret: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(b"csrf:");
    mac.update(&session_id.to_be_bytes());
    mac.update(nonce);
    hex::encode(mac.finalize().into_bytes())
}

/// Build a Set-Cookie header for the CSRF companion cookie. NOT
/// HttpOnly — the client-side JS in apiFetch must be able to read it
/// and echo the value in `X-CSRF-Token`. Still scoped Path=/ +
/// SameSite=Lax + Secure (when applicable) so it shares the session
/// cookie's protections.
pub fn set_csrf_cookie_header(value: &str, max_age_s: i64, secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!(
        "{name}={value}; Path=/; SameSite=Lax; Max-Age={max_age_s}{secure_attr}",
        name = crate::auth::csrf_cookie_name(secure),
    )
}

/// Clear the CSRF companion cookie at logout.
pub fn clear_csrf_cookie_header(secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!(
        "{name}=; Path=/; SameSite=Lax; Max-Age=0{secure_attr}",
        name = crate::auth::csrf_cookie_name(secure),
    )
}

/// Extract a named cookie value from a `Cookie:` header.
pub fn find_cookie<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    for chunk in header.split(';') {
        let trimmed = chunk.trim();
        if let Some((k, v)) = trimmed.split_once('=') {
            if k == name {
                return Some(v);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let secret = b"a-test-secret-32-bytes-or-larger!";
        let nonce = [7u8; 32];
        let value = build_value(42, &nonce, secret);
        let parsed = parse_value(&value, secret).unwrap();
        assert_eq!(parsed.0, 42);
        assert_eq!(parsed.1, nonce);
    }

    #[test]
    fn rejects_wrong_secret() {
        let nonce = [7u8; 32];
        let value = build_value(42, &nonce, b"secret-one-aaaaaaaaaaaaaaaaaaaaaaa");
        assert!(parse_value(&value, b"secret-two-bbbbbbbbbbbbbbbbbbbbbbb").is_none());
    }

    #[test]
    fn rejects_tampered_session_id() {
        let secret = b"sssssssssssssssssssssssssssssssss";
        let nonce = [7u8; 32];
        let mut value = build_value(42, &nonce, secret);
        value.replace_range(0..2, "99");
        assert!(parse_value(&value, secret).is_none());
    }

    #[test]
    fn find_cookie_handles_spaces_and_multiples() {
        let header = "first=a; cf_session=hello; last=b";
        assert_eq!(find_cookie(header, "cf_session"), Some("hello"));
        assert_eq!(find_cookie(header, "first"), Some("a"));
        assert_eq!(find_cookie(header, "missing"), None);
    }
}
