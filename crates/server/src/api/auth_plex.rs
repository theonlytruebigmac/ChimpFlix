//! `/auth/plex/*` — Plex OAuth (PIN flow) endpoints.
//!
//! Three intents flow through the same `start` / `poll` pair, picked
//! by the body of `/auth/plex/start`:
//!
//!   * **login**  (anonymous): poll yields a session if the resulting
//!                Plex identity is already linked to a local user;
//!                otherwise the response is `not_linked` and the
//!                browser shows "ask for an invite".
//!   * **signup** (anonymous, requires invite): poll provisions a
//!                fresh local user, links the Plex identity, consumes
//!                the invite, and issues a session.
//!   * **link**   (authenticated): poll attaches the Plex identity to
//!                the requesting user's account. No new session is
//!                issued; the existing one stays valid.
//!
//! The frontend never sees the raw Plex PIN id — `start` returns an
//! opaque `pin_handle` we generate, and `poll` looks the underlying
//! PIN id up on the server. That keeps a hostile script in another
//! tab from polling someone else's in-flight authorization just by
//! guessing PIN ids.

use std::time::Duration;

use axum::Extension;
use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use axum::response::IntoResponse;
use chimpflix_common::now_ms;
use chimpflix_library::{
    NewAuditEntry, UserRole, hash_invite_code, queries,
    queries::{UserAuthProvider, allocate_username_from_external},
};
use chimpflix_metadata::{PinPollResult, PlexUser};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::auth::{authed_response, issue_session};
use crate::api::error::ApiError;
use crate::auth::{AuthUser, MaybeAuthUser};
use crate::client_ip::EffectiveClientIp;
use crate::state::{AppState, PendingPlexPin, PlexPinIntent};

const PLEX_PROVIDER: &str = "plex";
/// How long we let an opaque PIN handle sit in the cache. Mirrors
/// Plex's `strong=true` PIN lifetime (~30 minutes); we use a slightly
/// shorter ceiling so a stale entry doesn't outlive the upstream PIN.
const PIN_HANDLE_TTL: Duration = Duration::from_secs(25 * 60);

// ─── /auth/plex/start ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum StartInput {
    /// Anonymous login attempt; resulting Plex identity must already
    /// be linked locally or the poll returns `not_linked`.
    Login,
    /// Anonymous signup via invite. The plaintext code is hashed
    /// server-side and bound to the resulting handle so a stolen
    /// handle can't be swapped onto a different invite mid-flow.
    Signup { invite_code: String },
    /// Attach to the currently signed-in account. The session is
    /// re-verified at poll time via the AuthUser extractor.
    Link,
}

#[derive(Debug, Serialize)]
pub struct StartResponse {
    /// Opaque token to pass back to `/auth/plex/poll`. Random,
    /// short-lived, server-side only.
    pub pin_handle: String,
    /// Where to open the Plex authorization page. The browser opens
    /// this in a new tab; we poll on this side while the user goes
    /// through Plex's UI.
    pub auth_url: String,
    /// User-visible 4-character code Plex showed. Surfaced in case the
    /// new tab gets blocked and we want a fallback "go to plex.tv/link
    /// and enter this code" UI; the standard flow ignores it.
    pub user_code: String,
    /// Seconds remaining on the PIN. We don't currently expose a
    /// countdown in the UI but this is the upper bound on how long
    /// polling makes sense.
    pub expires_in: i64,
}

pub async fn start(
    State(state): State<AppState>,
    MaybeAuthUser(user): MaybeAuthUser,
    Json(input): Json<StartInput>,
) -> Result<Json<StartResponse>, ApiError> {
    // Resolve intent + capture any cross-cutting prerequisites BEFORE
    // we burn a Plex PIN. Each branch fails fast on bad inputs so the
    // operator-visible Plex API quota doesn't tick up for refused
    // requests.
    let intent = match input {
        StartInput::Login => PlexPinIntent::Login,
        StartInput::Signup { invite_code } => {
            let raw = invite_code.trim();
            if raw.is_empty() {
                return Err(ApiError::validation("invite code is required for signup"));
            }
            if raw.len() > 256 {
                return Err(ApiError::validation("invite code is invalid"));
            }
            let code_hash = hash_invite_code(raw);
            let invite = queries::find_invite_by_code_hash(&state.pool, &code_hash)
                .await
                .map_err(ApiError::Internal)?
                .ok_or_else(|| ApiError::validation("invite code is invalid"))?;
            if invite.consumed_by.is_some() {
                return Err(ApiError::validation("invite code has already been used"));
            }
            if let Some(exp) = invite.expires_at {
                if exp < now_ms() {
                    return Err(ApiError::validation("invite code has expired"));
                }
            }
            PlexPinIntent::Signup {
                invite_code_hash: code_hash,
            }
        }
        StartInput::Link => {
            let u = user.as_ref().ok_or(ApiError::Unauthorized)?;
            PlexPinIntent::Link { user_id: u.id }
        }
    };

    let client = state
        .plex_oauth()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Plex OAuth init failed: {e:#}")))?;
    let pin = client.create_pin().await.map_err(ApiError::Internal)?;
    let pin_handle = random_handle();
    let pending = PendingPlexPin {
        plex_pin_id: pin.id,
        intent,
        expires_at: std::time::Instant::now() + PIN_HANDLE_TTL,
    };
    state.plex_pin_remember(pin_handle.clone(), pending).await;

    Ok(Json(StartResponse {
        pin_handle,
        auth_url: client.auth_url(&pin.code, None),
        user_code: pin.code,
        expires_in: pin.expires_in,
    }))
}

// ─── /auth/plex/poll ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PollInput {
    pub pin_handle: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PollResponse {
    /// User hasn't approved yet. Frontend polls again after the
    /// suggested cadence.
    Pending,
    /// PIN lifecycle ended without approval — either the user denied
    /// it or the timer ran out.
    Expired,
    /// The provided handle isn't known to us. Either it was already
    /// consumed by a successful poll, never existed, or has been
    /// garbage-collected.
    UnknownHandle,
    /// Plex accepted the approval but the resulting identity has no
    /// ChimpFlix account linked to it. Only emitted for the `login`
    /// intent — `signup` / `link` create the link as part of the
    /// flow.
    NotLinked { plex_username: String },
    /// Link intent only: the Plex identity is now attached to the
    /// requesting user's account. (Login + Signup intents bypass this
    /// variant — they return the classic `{ user }` payload from
    /// `authed_response` along with session cookies, so the frontend's
    /// existing post-login bootstrap handles them transparently.)
    Linked,
}

pub async fn poll(
    State(state): State<AppState>,
    MaybeAuthUser(user): MaybeAuthUser,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    headers: HeaderMap,
    Json(input): Json<PollInput>,
) -> Result<axum::response::Response, ApiError> {
    let Some(pending) = state.plex_pin_lookup(&input.pin_handle).await else {
        return Ok(Json(PollResponse::UnknownHandle).into_response());
    };

    let client = state
        .plex_oauth()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("Plex OAuth init failed: {e:#}")))?;
    let result = client
        .poll_pin(pending.plex_pin_id)
        .await
        .map_err(ApiError::Internal)?;
    let token = match result {
        PinPollResult::Pending => return Ok(Json(PollResponse::Pending).into_response()),
        PinPollResult::Expired => {
            state.plex_pin_forget(&input.pin_handle).await;
            return Ok(Json(PollResponse::Expired).into_response());
        }
        PinPollResult::Ready(t) => t,
    };

    // We have a Plex token; resolve it to the underlying identity
    // exactly once and throw the token away.
    let plex_user = client
        .fetch_user(&token)
        .await
        .map_err(ApiError::Internal)?;
    state.plex_pin_forget(&input.pin_handle).await;

    let external_id = plex_user.id.to_string();
    match pending.intent {
        PlexPinIntent::Login => {
            finalize_login(&state, &headers, Some(ip), &plex_user, &external_id).await
        }
        PlexPinIntent::Signup { invite_code_hash } => {
            finalize_signup(
                &state,
                &headers,
                Some(ip),
                &plex_user,
                &external_id,
                &invite_code_hash,
            )
            .await
        }
        PlexPinIntent::Link { user_id } => {
            // Re-verify the requesting session matches the one that
            // initiated the link. If the user signed out between start
            // and poll, refuse rather than silently linking under the
            // wrong identity.
            let Some(current) = user else {
                return Err(ApiError::Unauthorized);
            };
            if current.id != user_id {
                return Err(ApiError::Unauthorized);
            }
            finalize_link(&state, &headers, user_id, &plex_user, &external_id).await
        }
    }
}

// ─── /auth/plex/link  (GET = list, DELETE = unlink) ────────────────────

#[derive(Debug, Serialize)]
pub struct LinkSummary {
    pub provider: String,
    pub external_username: Option<String>,
    pub external_email: Option<String>,
    pub linked_at: i64,
    pub last_login_at: Option<i64>,
}

impl From<UserAuthProvider> for LinkSummary {
    fn from(p: UserAuthProvider) -> Self {
        Self {
            provider: p.provider,
            external_username: p.external_username,
            external_email: p.external_email,
            linked_at: p.linked_at,
            last_login_at: p.last_login_at,
        }
    }
}

pub async fn list_links(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<LinkSummary>>, ApiError> {
    let links = queries::list_user_auth_providers(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(links.into_iter().map(LinkSummary::from).collect()))
}

#[derive(Debug, Serialize)]
pub struct UnlinkResponse {
    pub removed: bool,
}

pub async fn unlink(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
) -> Result<Json<UnlinkResponse>, ApiError> {
    // Safety: refuse to unlink if it would leave the user with no way
    // to sign in. Password-less Plex-only users have to set a local
    // password (via the forgot-password email flow) before unlinking,
    // OR link a second provider once we support more than one.
    let has_password = queries::user_has_password(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    if !has_password {
        let links = queries::list_user_auth_providers(&state.pool, user.id)
            .await
            .map_err(ApiError::Internal)?;
        // The Plex link is the one we're about to remove — every other
        // entry counts as a "fallback sign-in path". With no others +
        // no password, refusing the unlink keeps the user from
        // locking themselves out.
        let others = links.iter().filter(|l| l.provider != PLEX_PROVIDER).count();
        if others == 0 {
            return Err(ApiError::validation(
                "this is your only way to sign in — set a password (Settings → Account → \
                 \"Forgot password\") before unlinking Plex, or this account will be \
                 inaccessible",
            ));
        }
    }

    let removed = queries::delete_auth_provider(&state.pool, user.id, PLEX_PROVIDER)
        .await
        .map_err(ApiError::Internal)?;
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(user.id),
            action: "auth.plex.unlink".into(),
            target_kind: Some("user".into()),
            target_id: Some(user.id.to_string()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(Json(UnlinkResponse { removed: removed > 0 }))
}

// ─── Finalize helpers ──────────────────────────────────────────────────

async fn finalize_login(
    state: &AppState,
    headers: &HeaderMap,
    ip: Option<std::net::IpAddr>,
    plex_user: &PlexUser,
    external_id: &str,
) -> Result<axum::response::Response, ApiError> {
    let Some((user, link)) =
        queries::find_user_by_provider(&state.pool, PLEX_PROVIDER, external_id)
            .await
            .map_err(ApiError::Internal)?
    else {
        return Ok(Json(PollResponse::NotLinked {
            plex_username: plex_user.username.clone(),
        })
        .into_response());
    };
    let _ = queries::touch_auth_provider_login(&state.pool, link.id).await;
    let cookies = issue_session(state, &user, headers, ip).await?;
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        state,
        NewAuditEntry {
            actor_user_id: Some(user.id),
            action: "auth.plex.login".into(),
            target_kind: Some("user".into()),
            target_id: Some(user.id.to_string()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    // Re-use authed_response: same JSON shape as classic login so the
    // frontend's existing post-login bootstrap (auth context, prefs
    // hydration) works without forks.
    Ok(authed_response(
        axum::http::StatusCode::OK,
        user,
        cookies,
        state,
    ))
}

async fn finalize_signup(
    state: &AppState,
    headers: &HeaderMap,
    ip: Option<std::net::IpAddr>,
    plex_user: &PlexUser,
    external_id: &str,
    invite_code_hash: &str,
) -> Result<axum::response::Response, ApiError> {
    // Re-validate the invite *now* — between start and poll, an
    // operator could have revoked it, or someone else could have
    // consumed it.
    let invite = queries::find_invite_by_code_hash(&state.pool, invite_code_hash)
        .await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::validation("invite code is no longer valid"))?;
    if invite.consumed_by.is_some() {
        return Err(ApiError::validation("invite code has already been used"));
    }
    if let Some(exp) = invite.expires_at {
        if exp < now_ms() {
            return Err(ApiError::validation("invite code has expired"));
        }
    }
    // Refuse if this Plex identity is already linked to a different
    // ChimpFlix account — preserves the (provider, external_id)
    // uniqueness invariant cleanly instead of letting the INSERT 500.
    if let Some((existing, _)) =
        queries::find_user_by_provider(&state.pool, PLEX_PROVIDER, external_id)
            .await
            .map_err(ApiError::Internal)?
    {
        return Err(ApiError::Conflict(format!(
            "this Plex account is already linked to user '{}'",
            existing.username
        )));
    }

    let username = allocate_username_from_external(&state.pool, &plex_user.username)
        .await
        .map_err(ApiError::Internal)?;
    let user = queries::create_user_no_password(
        &state.pool,
        &username,
        UserRole::User,
        None,
        plex_user.email.as_deref().or(invite.email.as_deref()),
    )
    .await
    .map_err(|e| {
        let msg = format!("{e:#}");
        if msg.contains("UNIQUE constraint failed") {
            ApiError::Conflict("username or email already exists".into())
        } else {
            ApiError::Internal(e)
        }
    })?;
    let _ = queries::insert_auth_provider(
        &state.pool,
        user.id,
        PLEX_PROVIDER,
        external_id,
        plex_user.email.as_deref(),
        Some(plex_user.username.as_str()),
    )
    .await
    .map_err(ApiError::Internal)?;
    queries::consume_invite(&state.pool, invite_code_hash, user.id)
        .await
        .map_err(ApiError::Internal)?;
    let cookies = issue_session(state, &user, headers, ip).await?;
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        state,
        NewAuditEntry {
            actor_user_id: Some(user.id),
            action: "auth.plex.signup".into(),
            target_kind: Some("user".into()),
            target_id: Some(user.id.to_string()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(authed_response(
        axum::http::StatusCode::CREATED,
        user,
        cookies,
        state,
    ))
}

async fn finalize_link(
    state: &AppState,
    headers: &HeaderMap,
    user_id: i64,
    plex_user: &PlexUser,
    external_id: &str,
) -> Result<axum::response::Response, ApiError> {
    // Reject if the Plex identity is bound to a different user — we
    // don't want a silent steal of someone else's account by approving
    // their Plex PIN from a different session.
    if let Some((existing, _)) =
        queries::find_user_by_provider(&state.pool, PLEX_PROVIDER, external_id)
            .await
            .map_err(ApiError::Internal)?
    {
        if existing.id != user_id {
            return Err(ApiError::Conflict(format!(
                "this Plex account is already linked to user '{}'",
                existing.username
            )));
        }
        // Idempotent: linking the same account again is a no-op.
        return Ok(Json(PollResponse::Linked).into_response());
    }
    let _ = queries::insert_auth_provider(
        &state.pool,
        user_id,
        PLEX_PROVIDER,
        external_id,
        plex_user.email.as_deref(),
        Some(plex_user.username.as_str()),
    )
    .await
    .map_err(|e| {
        let msg = format!("{e:#}");
        if msg.contains("UNIQUE constraint failed") {
            ApiError::Conflict("a Plex account is already linked to this user".into())
        } else {
            ApiError::Internal(e)
        }
    })?;
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        state,
        NewAuditEntry {
            actor_user_id: Some(user_id),
            action: "auth.plex.link".into(),
            target_kind: Some("user".into()),
            target_id: Some(user_id.to_string()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(Json(PollResponse::Linked).into_response())
}

// ─── helpers ──────────────────────────────────────────────────────────

fn random_handle() -> String {
    let mut bytes = [0u8; 24];
    use rand_core::{OsRng, RngCore};
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
