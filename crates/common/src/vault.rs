//! Encrypted credential vault.
//!
//! Two surfaces sit on top of the same primitive:
//!   - **Named secrets** — store/retrieve by string name (`tmdb`, `tvdb`,
//!     `anilist`, `session_hmac`). Persistence lives in the `secrets` SQLite
//!     table; this module owns the crypto.
//!   - **Per-row encryption helper** — callers with their own column (e.g.
//!     webhook signing secrets) use [`Vault::encrypt`] / [`Vault::decrypt`]
//!     directly.
//!
//! Encryption is ChaCha20-Poly1305 with a 32-byte master key sourced from
//! the `CHIMPFLIX_SECRET_KEY` env var (hex-encoded, 64 chars). If the env
//! var is absent the vault enters **plaintext mode**: values are stored
//! verbatim with `nonce = None` as the on-disk signal. This keeps
//! `docker compose up` working on first boot; the server logs a loud
//! warning and prints a ready-to-paste key so the operator can harden.

use anyhow::{Context, Result, bail};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, KeyInit},
};
use rand_core::{OsRng, RngCore};

/// Env var holding the hex-encoded 32-byte master key.
pub const MASTER_KEY_ENV: &str = "CHIMPFLIX_SECRET_KEY";

const MASTER_KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;

/// A stored blob. `nonce` is `None` when the vault was in plaintext mode
/// at the time of write — the persistence layer maps that to a `NULL`
/// nonce column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedBlob {
    pub value: Vec<u8>,
    pub nonce: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct Vault {
    cipher: Option<ChaCha20Poly1305>,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("encrypted", &self.cipher.is_some())
            .finish()
    }
}

impl Vault {
    /// Load the master key from `CHIMPFLIX_SECRET_KEY` if set; otherwise
    /// build a plaintext-mode vault. Returns `(vault, encrypted)` so the
    /// caller can emit the appropriate boot log.
    ///
    /// Errors only if the env var is present but malformed — a missing
    /// env var is the documented plaintext-fallback path, not an error.
    pub fn from_env() -> Result<(Self, bool)> {
        match std::env::var(MASTER_KEY_ENV) {
            Ok(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    Ok((Self::plaintext(), false))
                } else {
                    Ok((Self::from_hex_key(trimmed)?, true))
                }
            }
            Err(_) => Ok((Self::plaintext(), false)),
        }
    }

    /// Build from a hex-encoded 32-byte key.
    pub fn from_hex_key(hex: &str) -> Result<Self> {
        let bytes = hex::decode(hex.trim())
            .with_context(|| format!("{MASTER_KEY_ENV} must be hex-encoded"))?;
        Self::with_key(&bytes)
    }

    /// Build from raw 32-byte key material.
    pub fn with_key(key: &[u8]) -> Result<Self> {
        if key.len() != MASTER_KEY_BYTES {
            bail!(
                "vault key must be exactly {MASTER_KEY_BYTES} bytes (got {})",
                key.len()
            );
        }
        let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
        Ok(Self {
            cipher: Some(cipher),
        })
    }

    /// Plaintext-mode vault. Stored values pass through unencrypted.
    pub fn plaintext() -> Self {
        Self { cipher: None }
    }

    /// `true` when a master key is present and stored values are encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.cipher.is_some()
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedBlob> {
        match &self.cipher {
            Some(cipher) => {
                let mut nonce_bytes = [0u8; NONCE_BYTES];
                OsRng.fill_bytes(&mut nonce_bytes);
                let nonce = Nonce::from_slice(&nonce_bytes);
                let value = cipher
                    .encrypt(nonce, plaintext)
                    .map_err(|e| anyhow::anyhow!("vault encrypt failed: {e}"))?;
                Ok(EncryptedBlob {
                    value,
                    nonce: Some(nonce_bytes.to_vec()),
                })
            }
            None => Ok(EncryptedBlob {
                value: plaintext.to_vec(),
                nonce: None,
            }),
        }
    }

    pub fn decrypt(&self, blob: &EncryptedBlob) -> Result<Vec<u8>> {
        match (&self.cipher, &blob.nonce) {
            (Some(cipher), Some(nonce_bytes)) => {
                if nonce_bytes.len() != NONCE_BYTES {
                    bail!(
                        "vault nonce must be {NONCE_BYTES} bytes (got {})",
                        nonce_bytes.len()
                    );
                }
                let nonce = Nonce::from_slice(nonce_bytes);
                cipher
                    .decrypt(nonce, blob.value.as_slice())
                    .map_err(|e| anyhow::anyhow!("vault decrypt failed: {e}"))
            }
            (None, None) => Ok(blob.value.clone()),
            // The mismatched-mode paths fire when the operator added or
            // removed CHIMPFLIX_SECRET_KEY after data was already written.
            // Surface a specific message so they can recover deliberately
            // (re-enter the credential after fixing the env).
            (Some(_), None) => bail!(
                "vault is in encrypted mode but stored blob has no nonce \
                 (was written before {MASTER_KEY_ENV} was set?)"
            ),
            (None, Some(_)) => bail!(
                "vault is in plaintext mode but stored blob has a nonce \
                 (was written while {MASTER_KEY_ENV} was set?)"
            ),
        }
    }

    pub fn encrypt_str(&self, plaintext: &str) -> Result<EncryptedBlob> {
        self.encrypt(plaintext.as_bytes())
    }

    pub fn decrypt_str(&self, blob: &EncryptedBlob) -> Result<String> {
        let bytes = self.decrypt(blob)?;
        String::from_utf8(bytes).context("decrypted secret was not valid UTF-8")
    }
}

/// Generate a fresh hex-encoded master key for the bootstrap "paste this
/// into your env" helper.
pub fn generate_master_key_hex() -> String {
    let mut bytes = [0u8; MASTER_KEY_BYTES];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encrypted() {
        let vault = Vault::with_key(&[7u8; 32]).unwrap();
        assert!(vault.is_encrypted());
        let blob = vault.encrypt_str("hunter2").unwrap();
        assert!(blob.nonce.is_some());
        assert_ne!(blob.value, b"hunter2");
        assert_eq!(vault.decrypt_str(&blob).unwrap(), "hunter2");
    }

    #[test]
    fn nonces_are_unique_across_writes() {
        let vault = Vault::with_key(&[0u8; 32]).unwrap();
        let a = vault.encrypt_str("same").unwrap();
        let b = vault.encrypt_str("same").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.value, b.value);
    }

    #[test]
    fn roundtrip_plaintext() {
        let vault = Vault::plaintext();
        assert!(!vault.is_encrypted());
        let blob = vault.encrypt_str("hunter2").unwrap();
        assert!(blob.nonce.is_none());
        assert_eq!(blob.value, b"hunter2");
        assert_eq!(vault.decrypt_str(&blob).unwrap(), "hunter2");
    }

    #[test]
    fn mismatched_mode_fails_loudly() {
        let enc = Vault::with_key(&[1u8; 32]).unwrap();
        let plain = Vault::plaintext();
        let enc_blob = enc.encrypt_str("x").unwrap();
        let plain_blob = plain.encrypt_str("x").unwrap();
        assert!(plain.decrypt(&enc_blob).is_err());
        assert!(enc.decrypt(&plain_blob).is_err());
    }

    #[test]
    fn wrong_key_decrypt_fails() {
        let a = Vault::with_key(&[1u8; 32]).unwrap();
        let b = Vault::with_key(&[2u8; 32]).unwrap();
        let blob = a.encrypt_str("x").unwrap();
        assert!(b.decrypt(&blob).is_err());
    }

    #[test]
    fn from_hex_key_round_trips_with_generated_key() {
        let key_hex = generate_master_key_hex();
        let vault = Vault::from_hex_key(&key_hex).unwrap();
        let blob = vault.encrypt_str("x").unwrap();
        let again = Vault::from_hex_key(&key_hex).unwrap();
        assert_eq!(again.decrypt_str(&blob).unwrap(), "x");
    }

    #[test]
    fn from_hex_key_rejects_wrong_length() {
        assert!(Vault::from_hex_key("deadbeef").is_err());
    }

    #[test]
    fn from_hex_key_rejects_non_hex() {
        assert!(Vault::from_hex_key(&"z".repeat(64)).is_err());
    }

    #[test]
    fn generate_master_key_is_64_hex_chars() {
        let k = generate_master_key_hex();
        assert_eq!(k.len(), 64);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(generate_master_key_hex(), k);
    }
}
