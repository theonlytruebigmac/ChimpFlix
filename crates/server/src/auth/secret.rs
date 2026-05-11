//! Loads (or generates on first run) the long-lived HMAC secret used to
//! sign session cookies.
//!
//! Resolution order:
//!   1. `SESSION_SECRET` env var (hex-encoded, ≥ 32 bytes / 64 hex chars).
//!   2. `${DATA_DIR}/session-secret` file (hex-encoded).
//!   3. Generate 32 random bytes, persist to the file (mode 0600 on unix).

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use tracing::{info, warn};

use crate::auth::password::fill_random;

pub fn load_or_generate(data_dir: &Path) -> Result<Vec<u8>> {
    if let Ok(val) = std::env::var("SESSION_SECRET") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            let bytes = hex::decode(trimmed).context("SESSION_SECRET must be hex-encoded")?;
            if bytes.len() < 32 {
                bail!("SESSION_SECRET must be at least 32 bytes (64 hex characters)");
            }
            info!("session secret loaded from SESSION_SECRET");
            return Ok(bytes);
        }
    }

    let path = data_dir.join("session-secret");
    if let Ok(content) = std::fs::read_to_string(&path) {
        let bytes = hex::decode(content.trim())
            .with_context(|| format!("{} is not valid hex", path.display()))?;
        if bytes.len() < 32 {
            bail!("{} contains a secret shorter than 32 bytes", path.display());
        }
        info!(path = %path.display(), "session secret loaded from disk");
        return Ok(bytes);
    }

    let mut bytes = [0u8; 32];
    fill_random(&mut bytes)?;
    let hex_value = hex::encode(bytes);
    persist(&path, &hex_value)?;
    warn!(path = %path.display(), "generated new session secret on first run");
    Ok(bytes.to_vec())
}

#[cfg(unix)]
fn persist(path: &Path, value: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    f.write_all(value.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn persist(path: &Path, value: &str) -> Result<()> {
    std::fs::write(path, value).with_context(|| format!("write {}", path.display()))
}
