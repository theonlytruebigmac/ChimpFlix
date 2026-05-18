//! TOTP (RFC 6238) helpers + recovery-code generation.
//!
//! All TOTP math goes through `totp-rs` so the parameters (SHA-1, 6
//! digits, 30s step) match what every authenticator app expects. We
//! only own the secret-storage shape (encrypt at rest via the vault),
//! the recovery-code format (hex hyphenated, hashed at rest), and the
//! short-lived "TOTP challenge" token used by the two-step login flow.

use anyhow::{Context, Result};
use chimpflix_common::{EncryptedBlob, Vault};
use hmac::{Hmac, Mac};
use qrcode::QrCode;
use qrcode::render::svg;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use totp_rs::{Algorithm, Secret, TOTP};

/// RFC 6238 default: SHA-1, 6 digits, 30-second period. Don't change
/// without a forced re-enrollment for every user — authenticator apps
/// bake these into the secret encoding.
const TOTP_DIGITS: usize = 6;
const TOTP_STEP_SECS: u64 = 30;
/// Tolerate one step in either direction (±30s) to absorb clock skew.
const TOTP_SKEW: u8 = 1;

/// Length of the raw TOTP secret in bytes. 20 bytes (160 bits) matches
/// the RFC 6238 reference + what Google Authenticator emits.
const SECRET_BYTES: usize = 20;

/// Number of recovery codes generated per enrollment.
pub const RECOVERY_CODE_COUNT: usize = 10;

/// TOTP challenge tokens (issued after password success, redeemed by
/// the 2fa/login endpoint) live this long. Long enough for the user to
/// fish out their phone, short enough that a leaked token can't be
/// reused tomorrow.
pub const CHALLENGE_TTL_SECS: i64 = 5 * 60;

pub struct EnrollmentMaterial {
    /// Plaintext base32 of the secret — surfaced to the user so they can
    /// manually type it into their authenticator if QR scanning fails.
    pub secret_b32: String,
    /// `otpauth://totp/...` provisioning URI for QR rendering.
    pub otpauth_uri: String,
    /// Inline `data:image/svg+xml;base64,…` for the otpauth_uri rendered
    /// as a QR code. The frontend drops this straight into an <img src>
    /// so we don't need a client-side QR library.
    pub qr_data_url: String,
    /// Encrypted secret + nonce ready to persist via [`upsert_user_totp`].
    pub blob: EncryptedBlob,
}

pub fn generate_enrollment(
    vault: &Vault,
    issuer: &str,
    account_label: &str,
) -> Result<EnrollmentMaterial> {
    let mut raw = [0u8; SECRET_BYTES];
    OsRng.fill_bytes(&mut raw);
    let secret = Secret::Raw(raw.to_vec());
    let secret_b32 = secret
        .to_encoded()
        .to_string()
        .trim_end_matches('=')
        .to_string();

    let totp = build_totp_with_meta(&raw, issuer, account_label)?;
    let otpauth_uri = totp.get_url();
    let qr_data_url = render_qr_data_url(&otpauth_uri)?;

    let blob = vault
        .encrypt_str(&secret_b32)
        .context("encrypt TOTP secret")?;

    Ok(EnrollmentMaterial {
        secret_b32,
        otpauth_uri,
        qr_data_url,
        blob,
    })
}

/// Render `text` as a QR code, return a `data:image/svg+xml;base64,…` URL.
/// Embedded base64 so the frontend can drop it straight into `<img src>`
/// without any extra JS / CSP allowances.
fn render_qr_data_url(text: &str) -> Result<String> {
    let code = QrCode::new(text.as_bytes()).context("build QR code")?;
    let svg = code
        .render::<svg::Color<'_>>()
        // Quiet zone of 2 modules + 4×4 px per module — large enough
        // for phone cameras to lock on quickly. Light-on-dark to match
        // the surrounding admin panel; the spec only requires
        // background lighter than foreground.
        .quiet_zone(true)
        .min_dimensions(180, 180)
        .dark_color(svg::Color("#0a0a0a"))
        .light_color(svg::Color("#ffffff"))
        .build();
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
    Ok(format!("data:image/svg+xml;base64,{b64}"))
}

/// Decrypt the persisted blob back to the base32 secret string.
pub fn decrypt_secret(vault: &Vault, secret_enc: &[u8], nonce: Option<&[u8]>) -> Result<String> {
    let blob = EncryptedBlob {
        value: secret_enc.to_vec(),
        nonce: nonce.map(|n| n.to_vec()),
    };
    vault.decrypt_str(&blob).context("decrypt TOTP secret")
}

/// Verify a 6-digit code against the stored secret. ±1 step tolerance.
pub fn verify_code(secret_b32: &str, code: &str) -> Result<bool> {
    let trimmed = code.trim();
    if trimmed.len() != TOTP_DIGITS || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Ok(false);
    }
    let secret_bytes = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .context("decode TOTP secret base32")?;
    // `otpauth` feature requires issuer + account_name in the constructor;
    // they're only used by .get_url(), so feed throwaway values.
    let totp = TOTP::new(
        Algorithm::SHA1,
        TOTP_DIGITS,
        TOTP_SKEW,
        TOTP_STEP_SECS,
        secret_bytes,
        None,
        String::new(),
    )
    .context("construct TOTP")?;
    Ok(totp.check_current(trimmed)?)
}

fn build_totp_with_meta(secret_raw: &[u8], issuer: &str, account: &str) -> Result<TOTP> {
    TOTP::new(
        Algorithm::SHA1,
        TOTP_DIGITS,
        TOTP_SKEW,
        TOTP_STEP_SECS,
        secret_raw.to_vec(),
        Some(issuer.to_string()),
        account.to_string(),
    )
    .map_err(Into::into)
}

// ---------------------------------------------------------------------------
// Recovery codes — random hex strings shown to user once, hashed at rest.
// ---------------------------------------------------------------------------

/// Generate `count` fresh recovery codes. Returns a `Vec` of
/// `(plaintext, sha256_hex)` tuples — the plaintext is shown to the
/// user exactly once, the hash is what we persist.
pub fn generate_recovery_codes(count: usize) -> Vec<(String, String)> {
    (0..count)
        .map(|_| {
            let mut buf = [0u8; 8];
            OsRng.fill_bytes(&mut buf);
            // 16-hex-char code, split into 4-4-4-4 for readability.
            let hex = format!(
                "{:04x}-{:04x}-{:04x}-{:04x}",
                u16::from_be_bytes([buf[0], buf[1]]),
                u16::from_be_bytes([buf[2], buf[3]]),
                u16::from_be_bytes([buf[4], buf[5]]),
                u16::from_be_bytes([buf[6], buf[7]]),
            );
            let hash = hash_recovery_code(&hex);
            (hex, hash)
        })
        .collect()
}

/// Hash a recovery code for storage / lookup. Trims + lowercases first
/// so user re-entry is case-insensitive (the codes contain only hex
/// digits + hyphens, both case-insensitive).
pub fn hash_recovery_code(code: &str) -> String {
    let normalized = code.trim().to_ascii_lowercase();
    hex::encode(Sha256::digest(normalized.as_bytes()))
}

// ---------------------------------------------------------------------------
// TOTP-challenge tokens — short-lived HMAC-signed handoff between login
// step 1 (password) and step 2 (TOTP code).
// ---------------------------------------------------------------------------

type HmacSha256 = Hmac<Sha256>;

/// Mint a TOTP-challenge token. Format: `{user_id}:{exp_ms}:{hmac_hex}`.
/// Signed with the same secret used for session cookies — rotating the
/// secret invalidates outstanding challenges, which is the desired
/// behavior since you'd want to invalidate sessions too.
pub fn build_challenge(user_id: i64, exp_ms: i64, secret: &[u8]) -> String {
    let payload = format!("{user_id}:{exp_ms}");
    let sig = sign(&payload, secret);
    format!("{payload}:{}", hex::encode(sig))
}

pub fn parse_challenge(value: &str, secret: &[u8], now_ms: i64) -> Option<i64> {
    let mut parts = value.splitn(3, ':');
    let user_id: i64 = parts.next()?.parse().ok()?;
    let exp_ms: i64 = parts.next()?.parse().ok()?;
    let sig_hex = parts.next()?;
    let sig = hex::decode(sig_hex).ok()?;

    let payload = format!("{user_id}:{exp_ms}");
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(payload.as_bytes());
    mac.verify_slice(&sig).ok()?;

    if exp_ms < now_ms {
        return None;
    }
    Some(user_id)
}

fn sign(payload: &str, secret: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_round_trip() {
        let secret = b"32-byte-test-secret-padding-1234";
        let now = 1_000_000;
        let token = build_challenge(42, now + 30_000, secret);
        assert_eq!(parse_challenge(&token, secret, now), Some(42));
    }

    #[test]
    fn challenge_rejects_expired() {
        let secret = b"32-byte-test-secret-padding-1234";
        let token = build_challenge(42, 1_000, secret);
        assert!(parse_challenge(&token, secret, 5_000).is_none());
    }

    #[test]
    fn challenge_rejects_tampering() {
        let secret = b"32-byte-test-secret-padding-1234";
        let mut token = build_challenge(42, 1_000_000_000, secret);
        // Flip a byte in the user-id portion.
        token.replace_range(0..2, "99");
        assert!(parse_challenge(&token, secret, 0).is_none());
    }

    #[test]
    fn recovery_codes_are_unique_and_normalised() {
        let codes = generate_recovery_codes(10);
        let plain: std::collections::HashSet<_> = codes.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(plain.len(), 10);
        for (p, h) in &codes {
            assert_eq!(hash_recovery_code(p), *h);
            // Case-insensitive: uppercasing must hash to the same value.
            assert_eq!(hash_recovery_code(&p.to_uppercase()), *h);
        }
    }
}
