//! Authentication: passwords, signed session cookies, request extractor.

pub mod cookie;
pub mod password;
pub mod secret;

mod extractor;

pub use extractor::{AuthUser, OwnerAuth};

use std::sync::Arc;

/// Session lifetime (30 days).
pub const SESSION_MAX_AGE_S: i64 = 30 * 24 * 3600;

/// Cookie name carrying the signed session reference.
pub const COOKIE_NAME: &str = "cf_session";

#[derive(Clone)]
pub struct AuthConfig {
    pub session_secret: Arc<Vec<u8>>,
    /// Whether to set the `Secure` cookie flag (HTTPS-only).
    pub cookie_secure: bool,
}
