//! Shared types and helpers across ChimpFlix crates.
//!
//! Intentionally tiny — grow it only when two other crates need the same
//! thing.

pub mod time;
pub mod vault;

use serde::Serialize;
use thiserror::Error;

pub use time::now_ms;
pub use vault::{EncryptedBlob, MASTER_KEY_ENV, Vault, generate_master_key_hex};

/// Top-level error type. Crate-specific errors convert into this for the
/// public API surface.
#[derive(Debug, Error)]
pub enum Error {
    #[error("not found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("validation failed: {0}")]
    Validation(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// User-Agent string for outbound HTTP (TMDB, etc.).
pub const USER_AGENT: &str = concat!("ChimpFlix/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Serialize)]
pub struct VersionInfo {
    pub version: &'static str,
    pub git_sha: Option<&'static str>,
}

pub const VERSION: VersionInfo = VersionInfo {
    version: env!("CARGO_PKG_VERSION"),
    git_sha: option_env!("GIT_SHA"),
};
