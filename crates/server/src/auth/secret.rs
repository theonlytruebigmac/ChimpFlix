//! Loads (or generates on first run) the long-lived HMAC secret used to
//! sign session cookies.
//!
//! As of Phase 12.5 the secret lives in the credential vault under the
//! name `session_hmac`. On first boot we migrate any pre-existing value
//! from the legacy locations:
//!
//!   1. `SESSION_SECRET` env var (hex-encoded, ≥ 32 bytes / 64 hex chars).
//!   2. `${DATA_DIR}/session-secret` file (hex-encoded; deleted after
//!      successful import).
//!
//! If nothing is found we generate 32 random bytes, store them in the
//! vault, and warn that a brand-new secret was provisioned.

use std::path::Path;

use anyhow::{Context, Result, bail};
use chimpflix_common::Vault;
use chimpflix_library::queries;
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::auth::password::fill_random;

const VAULT_KEY: &str = "session_hmac";
const LEGACY_FILE_NAME: &str = "session-secret";

pub async fn load_or_migrate(pool: &SqlitePool, vault: &Vault, data_dir: &Path) -> Result<Vec<u8>> {
    if let Some(hex_str) = queries::vault_get(pool, vault, VAULT_KEY).await? {
        let bytes = decode_secret_hex(hex_str.trim())?;
        info!("session secret loaded from credential vault");
        return Ok(bytes);
    }

    if let Ok(raw) = std::env::var("SESSION_SECRET") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let bytes = decode_secret_hex(trimmed).context("SESSION_SECRET")?;
            queries::vault_set(pool, vault, VAULT_KEY, trimmed, None).await?;
            info!("imported SESSION_SECRET from env into credential vault");
            return Ok(bytes);
        }
    }

    let legacy = data_dir.join(LEGACY_FILE_NAME);
    if legacy.exists() {
        let content = std::fs::read_to_string(&legacy)
            .with_context(|| format!("read {}", legacy.display()))?;
        let trimmed = content.trim();
        let bytes = decode_secret_hex(trimmed).with_context(|| format!("{}", legacy.display()))?;
        queries::vault_set(pool, vault, VAULT_KEY, trimmed, None).await?;
        match std::fs::remove_file(&legacy) {
            Ok(()) => info!(
                path = %legacy.display(),
                "migrated session secret to credential vault and deleted legacy file"
            ),
            Err(e) => warn!(
                path = %legacy.display(),
                error = %e,
                "migrated session secret to credential vault but failed to delete legacy file \
                 (it will be re-imported on next boot only if the vault row is removed)"
            ),
        }
        return Ok(bytes);
    }

    let mut bytes = [0u8; 32];
    fill_random(&mut bytes)?;
    let hex_value = hex::encode(bytes);
    queries::vault_set(pool, vault, VAULT_KEY, &hex_value, None).await?;
    warn!("generated a new session HMAC secret and stored it in the credential vault");
    Ok(bytes.to_vec())
}

fn decode_secret_hex(value: &str) -> Result<Vec<u8>> {
    let bytes = hex::decode(value).context("session HMAC must be hex-encoded")?;
    if bytes.len() < 32 {
        bail!("session HMAC must be at least 32 bytes (64 hex characters)");
    }
    Ok(bytes)
}
