//! Authentication: passwords, signed session cookies, request extractor.

pub mod cast_token;
pub mod cookie;
pub mod password;
pub mod secret;

mod extractor;

pub use extractor::{AdminAuth, AuthUser, MaybeAuthUser, OwnerAuth, StreamAuthUser};

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

/// Session absolute lifetime (30 days). After this, the cookie expires
/// regardless of activity.
pub const SESSION_MAX_AGE_S: i64 = 30 * 24 * 3600;

/// Session idle timeout (14 days since `last_seen_at`). A laptop you
/// haven't touched in two weeks shouldn't still be holding a live
/// session — even though the absolute cap above hasn't kicked in.
/// The extractor checks this on every request and rejects sessions
/// older than this window. Wakes-up are noticed via `touch_session`
/// updating `last_seen_at` on every authenticated request.
pub const SESSION_IDLE_TIMEOUT_S: i64 = 14 * 24 * 3600;

/// Cookie name carrying the signed session reference. Selected at
/// request-time based on whether the deployment is HTTPS-fronted:
///   * HTTPS → `__Host-cf_session` — the `__Host-` prefix is a
///     browser-enforced contract that the cookie MUST be `Secure`,
///     MUST have `Path=/`, and MUST NOT have a `Domain` attribute. It
///     also forbids overwriting from any other host (mitigates
///     subdomain takeover and cross-subdomain cookie smuggling).
///   * HTTP (dev / LAN-only) → `cf_session` (legacy name, no prefix
///     since `__Host-` requires `Secure` which browsers won't honor
///     over plain HTTP).
///
/// All callsites should route through [`cookie_name`] rather than
/// hardcoding either string so dev/prod stay in lock-step.
pub fn cookie_name(secure: bool) -> &'static str {
    if secure {
        "__Host-cf_session"
    } else {
        "cf_session"
    }
}

/// Header name carrying the double-submit CSRF token. Clients read the
/// non-HttpOnly `cf_csrf` cookie (also issued at login) and echo its
/// value here on every state-changing request. The CSRF middleware
/// verifies cookie == header; mismatch ⇒ 403. This is the second
/// layer behind SameSite=Lax + Origin/Referer.
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

/// Returns the cookie name for the CSRF companion cookie, mirroring
/// [`cookie_name`]'s HTTPS-aware `__Host-` prefix logic. Two cookies
/// (session + csrf) stay in sync about whether they're locked to a
/// secure origin.
pub fn csrf_cookie_name(secure: bool) -> &'static str {
    if secure { "__Host-cf_csrf" } else { "cf_csrf" }
}

#[derive(Clone)]
pub struct AuthConfig {
    pub session_secret: Arc<Vec<u8>>,
    /// Whether to set the `Secure` cookie flag (HTTPS-only). When true
    /// the cookie also uses the `__Host-` prefix; see [`cookie_name`].
    pub cookie_secure: bool,
}
