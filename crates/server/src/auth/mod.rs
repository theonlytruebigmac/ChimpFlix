//! Authentication: passwords, signed session cookies, request extractor.

pub mod cookie;
pub mod password;
pub mod secret;

mod extractor;

pub use extractor::{AdminAuth, AuthUser, OwnerAuth};

use chimpflix_library::UserRole;
use std::sync::Arc;

/// Hierarchy guard: returns `Ok(())` if `actor` may modify the account
/// of someone currently at `target` role, else `Err(ApiError::Forbidden)`.
/// Rules:
///   - Owners may act on anyone (including other owners).
///   - Admins may act on users and other admins, but NEVER on owners.
///   - Users have no admin powers (the AdminAuth extractor already
///     rejects them before this is called).
///
/// Self-modification is not gated here — handlers that allow / forbid
/// self-actions check `actor.id == target.id` separately (e.g. "you
/// can't delete your own account" is a UI choice, "you can't demote
/// yourself" is a safety belt).
pub fn can_act_on(actor: UserRole, target: UserRole) -> Result<(), crate::api::error::ApiError> {
    match (actor, target) {
        // Owners are unrestricted within the hierarchy.
        (UserRole::Owner, _) => Ok(()),
        // Admins reach users and other admins, but never owners.
        (UserRole::Admin, UserRole::Owner) => Err(crate::api::error::ApiError::Forbidden),
        (UserRole::Admin, _) => Ok(()),
        // Plain users shouldn't reach this function at all; if they
        // somehow do, reject — defense in depth.
        (UserRole::User, _) => Err(crate::api::error::ApiError::Forbidden),
    }
}

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
