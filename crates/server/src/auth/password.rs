//! argon2id password hashing.
//!
//! Parameter choice: m=64 MiB, t=3, p=1. OWASP's 2024 floor is m=19 MiB
//! / t=2 (what `Argon2::default()` gave us); 64 MiB / t=3 is the
//! "preferred" cell on the same table and gives 3-4x more headroom
//! against future GPU/ASIC advances. The PHC hash string carries its
//! own parameters, so verify-then-rehash-on-login transparently
//! migrates accounts created under weaker params.

use std::sync::OnceLock;

use anyhow::Result;
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version};
use rand_core::{OsRng, RngCore};

/// Target parameter set for new hashes (and the threshold below which
/// `needs_rehash` triggers a re-hash on next successful login).
const ARGON2_MEMORY_KIB: u32 = 64 * 1024; // 64 MiB
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;

fn argon2_strong() -> Argon2<'static> {
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        None,
    )
    .expect("argon2 params constants are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

pub fn hash(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2_strong()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash password: {e}"))?
        .to_string();
    Ok(hash)
}

/// Returns true when the stored hash was produced with weaker
/// parameters than the current target. Callers (the login handler)
/// rehash on the next successful login so accounts created under the
/// old default get silently upgraded.
pub fn needs_rehash(hash_str: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash_str) else {
        return false;
    };
    // PHC encodes m=, t=, p= in the params section. Compare each;
    // missing values mean a non-Argon2id hash (legacy or other algo) —
    // treat as needing rehash so it gets upgraded on next login.
    let mut m: Option<u32> = None;
    let mut t: Option<u32> = None;
    let mut p: Option<u32> = None;
    for kv in parsed.params.iter() {
        let key = kv.0.as_str();
        if key == "m" {
            m = kv.1.decimal().ok();
        } else if key == "t" {
            t = kv.1.decimal().ok();
        } else if key == "p" {
            p = kv.1.decimal().ok();
        }
    }
    match (m, t, p) {
        (Some(m), Some(t), Some(p)) => {
            m < ARGON2_MEMORY_KIB || t < ARGON2_ITERATIONS || p < ARGON2_PARALLELISM
        }
        // Couldn't parse — be safe and rehash with the strong params.
        _ => true,
    }
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
/// hash; never panics. The verifier doesn't depend on the strong-params
/// configuration: the PHC hash string carries its own parameters and
/// argon2 re-derives the work from them, so a hash made under any
/// parameter set still verifies.
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
