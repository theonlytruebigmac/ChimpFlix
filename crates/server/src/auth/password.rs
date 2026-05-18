//! argon2id password hashing.

use std::sync::OnceLock;

use anyhow::Result;
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use rand_core::{OsRng, RngCore};

pub fn hash(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash password: {e}"))?
        .to_string();
    Ok(hash)
}

/// Lazily-initialized dummy hash for constant-time login responses.
/// The login handler verifies user-supplied passwords against THIS hash
/// when the username doesn't exist — without it, the timing of "user not
/// found" (skip argon2) vs "user found" (run argon2) leaks whether a
/// username is registered. We compute it once on first use; subsequent
/// calls just hand back the same string.
pub fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        // Random throwaway value — what's hashed doesn't matter, only
        // that the resulting string is a valid argon2 hash so verify()
        // takes the same path as a real one.
        let mut buf = [0u8; 32];
        OsRng.fill_bytes(&mut buf);
        hash(&hex::encode(buf)).expect("dummy hash generation must succeed")
    })
}

/// Constant-time verify. Returns false for any malformed or non-matching
/// hash; never panics.
pub fn verify(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Fill `buf` with cryptographically secure random bytes via OsRng.
/// `RngCore::fill_bytes` panics only if the OS RNG itself is broken,
/// which we treat as fatal.
pub fn fill_random(buf: &mut [u8]) -> Result<()> {
    let mut rng = OsRng;
    rng.fill_bytes(buf);
    Ok(())
}
