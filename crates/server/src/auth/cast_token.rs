//! Short-lived HMAC tokens that let a Chromecast receiver fetch
//! stream URLs without carrying the user's auth cookie.
//!
//! The Cast Web Sender hands the receiver an absolute URL; the
//! receiver fetches it directly from the server with no cookies, no
//! Origin, no CSRF token. That breaks every other auth path we have.
//! To plug the gap we mint a tiny token here, append it to the
//! manifest + segment URLs as `?ct=<token>`, and recognize it in the
//! stream extractor ([`super::extractor::StreamAuthUser`]).
//!
//! The token grants the same library access the user already has —
//! it's a transparent stand-in for the cookie, not a privilege
//! escalation. If stolen, the attacker can stream the user's library
//! until the token expires; this is the same blast radius as if they
//! had stolen the auth cookie itself, except the token carries no
//! admin powers (the stream endpoints aren't admin-gated, and the
//! cookie-required admin surfaces refuse to honour `?ct=`).
//!
//! Format on the wire (URL-safe base64, no padding):
//!
//! ```text
//!   payload = "{user_id}:{expires_at_ms}"
//!   token   = base64url(payload || ":" || hex(HMAC-SHA256(payload, session_secret)))
//! ```
//!
//! The same `session_secret` that signs the auth cookie signs cast
//! tokens — a server-secret rotation invalidates both surfaces in one
//! shot, which matches operator expectations.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chimpflix_common::now_ms;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Domain-separation prefix mixed into the HMAC for cast tokens.
///
/// Several token types (cast tokens here, the TOTP login challenge in
/// `crate::totp`) share the same `session_secret` and the same
/// `"{user_id}:{timestamp_ms}"` payload shape. Without a per-purpose
/// prefix, a token minted for one purpose verifies as the other —
/// e.g. a TOTP challenge string (returned in the login response before
/// 2FA is satisfied) could be replayed as a `?ct=` cast token,
/// bypassing 2FA on the stream surface. Prefixing the HMAC input binds
/// each token to its purpose. Keep this in sync with the matching
/// prefix in `totp.rs`.
const CAST_TOKEN_DOMAIN: &[u8] = b"cast:";

/// Default token TTL: 6 hours. Sized to outlast any single feature-
/// length film (most are <3h) plus a full TV-episode binge sitting,
/// while still expiring within the same evening so a stolen URL
/// from chat scrollback isn't reusable the next day. The earlier 1h
/// TTL hit a real bug: a 2-hour movie cast started at minute 0,
/// the token died at minute 60, and the receiver started getting
/// 401s on the next segment fetch mid-watch. The receiver UI just
/// shows a generic playback error in that case with no obvious
/// recovery, so we bias toward "token outlives the session" over
/// "token is as short as possible."
pub const DEFAULT_TTL_MS: i64 = 6 * 60 * 60 * 1000;

/// Mint a token granting `user_id` stream access until `expires_at_ms`
/// (absolute wall-clock ms since epoch). Callers typically pass
/// `now_ms() + DEFAULT_TTL_MS`.
pub fn mint(user_id: i64, expires_at_ms: i64, secret: &[u8]) -> String {
    let payload = format!("{user_id}:{expires_at_ms}");
    let sig = sign(&payload, secret);
    let raw = format!("{payload}:{}", hex::encode(sig));
    URL_SAFE_NO_PAD.encode(raw)
}

/// Verify a token and return `(user_id, expires_at_ms)` if it parses,
/// the HMAC matches, and it hasn't expired. Returns `None` on any
/// failure — the extractor should treat None as "fall back to cookie
/// auth" and return 401 if that also fails.
pub fn verify(token: &str, secret: &[u8]) -> Option<(i64, i64)> {
    let raw_bytes = URL_SAFE_NO_PAD.decode(token).ok()?;
    let raw = std::str::from_utf8(&raw_bytes).ok()?;
    let mut parts = raw.splitn(3, ':');
    let user_id_s = parts.next()?;
    let expires_s = parts.next()?;
    let sig_hex = parts.next()?;
    let user_id: i64 = user_id_s.parse().ok()?;
    let expires_at_ms: i64 = expires_s.parse().ok()?;
    let payload = format!("{user_id}:{expires_at_ms}");
    let sig = hex::decode(sig_hex).ok()?;
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(CAST_TOKEN_DOMAIN);
    mac.update(payload.as_bytes());
    mac.verify_slice(&sig).ok()?;
    if now_ms() > expires_at_ms {
        return None;
    }
    Some((user_id, expires_at_ms))
}

fn sign(payload: &str, secret: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(CAST_TOKEN_DOMAIN);
    mac.update(payload.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let secret = b"a-test-secret-32-bytes-or-larger!";
        let exp = now_ms() + 60_000;
        let token = mint(42, exp, secret);
        let (user_id, parsed_exp) = verify(&token, secret).unwrap();
        assert_eq!(user_id, 42);
        assert_eq!(parsed_exp, exp);
    }

    #[test]
    fn rejects_wrong_secret() {
        let exp = now_ms() + 60_000;
        let token = mint(42, exp, b"secret-one-aaaaaaaaaaaaaaaaaaaaaaa");
        assert!(verify(&token, b"secret-two-bbbbbbbbbbbbbbbbbbbbbbb").is_none());
    }

    #[test]
    fn rejects_expired_token() {
        let secret = b"sssssssssssssssssssssssssssssssss";
        let token = mint(42, now_ms() - 1, secret);
        assert!(verify(&token, secret).is_none());
    }

    #[test]
    fn rejects_tampered_user_id() {
        let secret = b"sssssssssssssssssssssssssssssssss";
        let exp = now_ms() + 60_000;
        let token = mint(42, exp, secret);
        // Decode → mutate → re-encode → verify must reject (HMAC
        // doesn't match a payload with a swapped user id).
        let raw = URL_SAFE_NO_PAD.decode(&token).unwrap();
        let s = String::from_utf8(raw).unwrap();
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        let mutated = format!("99:{}:{}", parts[1], parts[2]);
        let bad = URL_SAFE_NO_PAD.encode(mutated);
        assert!(verify(&bad, secret).is_none());
    }
}
