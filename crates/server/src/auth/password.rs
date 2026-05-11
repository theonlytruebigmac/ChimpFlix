//! argon2id password hashing.

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
