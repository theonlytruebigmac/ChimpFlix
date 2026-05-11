//! /api/v1/auth handlers: setup, login, logout, me, register, invites.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::{SET_COOKIE, USER_AGENT};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use chimpflix_common::now_ms;
use chimpflix_library::queries;
use chimpflix_library::{
    CreateInviteInput, Invite, LoginInput, RegisterInput, SetupInput, User, UserRole,
};
use serde::Serialize;
use tracing::info;

use crate::api::error::ApiError;
use crate::auth::{AuthUser, OwnerAuth, SESSION_MAX_AGE_S, cookie, password};
use crate::state::AppState;

const MIN_PASSWORD_LEN: usize = 8;
const MAX_USERNAME_LEN: usize = 64;

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    user: User,
}

#[derive(Debug, Serialize)]
pub struct AuthStatusResponse {
    pub setup_needed: bool,
}

pub async fn status(State(state): State<AppState>) -> Result<Json<AuthStatusResponse>, ApiError> {
    let setup_needed = queries::is_in_setup_mode(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(AuthStatusResponse { setup_needed }))
}

#[derive(Debug, Serialize)]
pub struct InviteResponse {
    invite: Invite,
}

#[derive(Debug, Serialize)]
pub struct InvitesListResponse {
    invites: Vec<Invite>,
}

// ---------------------------------------------------------------------------
// Setup (first-run only)
// ---------------------------------------------------------------------------

pub async fn setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<SetupInput>,
) -> Result<impl IntoResponse, ApiError> {
    if !queries::is_in_setup_mode(&state.pool)
        .await
        .map_err(ApiError::Internal)?
    {
        return Err(ApiError::Forbidden);
    }
    validate_username(&input.username)?;
    validate_password(&input.password)?;

    let hash = password::hash(&input.password).map_err(ApiError::Internal)?;
    let user = queries::complete_setup(
        &state.pool,
        input.username.trim(),
        &hash,
        input.display_name.as_deref(),
    )
    .await
    .map_err(ApiError::Internal)?;

    let cookie_value = issue_session(&state, &user, &headers).await?;
    info!(user_id = user.id, "setup complete");
    Ok(authed_response(
        StatusCode::CREATED,
        user,
        cookie_value,
        &state,
    ))
}

// ---------------------------------------------------------------------------
// Login / logout / me
// ---------------------------------------------------------------------------

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<LoginInput>,
) -> Result<impl IntoResponse, ApiError> {
    if input.username.is_empty() || input.password.is_empty() {
        return Err(ApiError::validation("username and password required"));
    }

    let Some(record) = queries::find_user_with_secret_by_username(&state.pool, &input.username)
        .await
        .map_err(ApiError::Internal)?
    else {
        return Err(invalid_credentials());
    };

    if record.user.username == "_default"
        || !password::verify(&input.password, &record.password_hash)
    {
        return Err(invalid_credentials());
    }

    let cookie_value = issue_session(&state, &record.user, &headers).await?;
    info!(user_id = record.user.id, "login");
    Ok(authed_response(
        StatusCode::OK,
        record.user,
        cookie_value,
        &state,
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    // Delete the *current* session row, then clear the cookie.
    let cookie_header = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if let Some(raw) = cookie::find_cookie(cookie_header, crate::auth::COOKIE_NAME) {
        if let Some((session_id, _)) = cookie::parse_value(raw, &state.auth.session_secret) {
            queries::delete_session(&state.pool, session_id)
                .await
                .map_err(ApiError::Internal)?;
        }
    }
    info!(user_id = user.id, "logout");
    let clear = cookie::clear_cookie_header(state.auth.cookie_secure);
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, HeaderValue::from_str(&clear).unwrap());
    Ok(response)
}

pub async fn me(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<AuthResponse>, ApiError> {
    let full = queries::find_user_by_id(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::Unauthorized)?;
    Ok(Json(AuthResponse { user: full }))
}

// ---------------------------------------------------------------------------
// Register (with invite)
// ---------------------------------------------------------------------------

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RegisterInput>,
) -> Result<impl IntoResponse, ApiError> {
    if input.code.trim().is_empty() {
        return Err(ApiError::validation("invite code is required"));
    }
    validate_username(&input.username)?;
    validate_password(&input.password)?;

    let invite = queries::find_invite_by_code(&state.pool, input.code.trim())
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

    let hash = password::hash(&input.password).map_err(ApiError::Internal)?;
    let user = queries::create_user(
        &state.pool,
        input.username.trim(),
        &hash,
        UserRole::User,
        input.display_name.as_deref(),
    )
    .await
    .map_err(|e| {
        // Surface unique-violation as a 409.
        let msg = format!("{e:#}");
        if msg.contains("UNIQUE constraint failed") {
            ApiError::Conflict("username already exists".into())
        } else {
            ApiError::Internal(e)
        }
    })?;

    queries::consume_invite(&state.pool, input.code.trim(), user.id)
        .await
        .map_err(ApiError::Internal)?;

    let cookie_value = issue_session(&state, &user, &headers).await?;
    info!(user_id = user.id, "register");
    Ok(authed_response(
        StatusCode::CREATED,
        user,
        cookie_value,
        &state,
    ))
}

// ---------------------------------------------------------------------------
// Invites (owner-only)
// ---------------------------------------------------------------------------

pub async fn list_invites(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<InvitesListResponse>, ApiError> {
    let invites = queries::list_invites(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(InvitesListResponse { invites }))
}

pub async fn create_invite(
    State(state): State<AppState>,
    OwnerAuth(user): OwnerAuth,
    Json(input): Json<CreateInviteInput>,
) -> Result<(StatusCode, Json<InviteResponse>), ApiError> {
    let expires_at = input.expires_in_seconds.map(|s| now_ms() + s.max(0) * 1000);
    let mut buf = [0u8; 16];
    password::fill_random(&mut buf).map_err(ApiError::Internal)?;
    let code = hex::encode(buf);

    let invite = queries::create_invite(&state.pool, &code, user.id, expires_at)
        .await
        .map_err(ApiError::Internal)?;
    Ok((StatusCode::CREATED, Json(InviteResponse { invite })))
}

pub async fn revoke_invite(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(code): Path<String>,
) -> Result<StatusCode, ApiError> {
    let revoked = queries::revoke_invite(&state.pool, &code)
        .await
        .map_err(ApiError::Internal)?;
    if revoked {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn validate_username(name: &str) -> Result<(), ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::validation("username is required"));
    }
    if trimmed.len() > MAX_USERNAME_LEN {
        return Err(ApiError::validation(format!(
            "username must be at most {MAX_USERNAME_LEN} characters"
        )));
    }
    if trimmed.starts_with('_') {
        return Err(ApiError::validation(
            "usernames starting with underscore are reserved",
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(ApiError::validation(
            "usernames may only contain letters, digits, dashes, dots, and underscores",
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(ApiError::validation(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters",
        )));
    }
    Ok(())
}

fn invalid_credentials() -> ApiError {
    ApiError::validation("invalid credentials")
}

async fn issue_session(
    state: &AppState,
    user: &User,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let mut nonce = [0u8; 32];
    password::fill_random(&mut nonce).map_err(ApiError::Internal)?;
    let expires_at = now_ms() + SESSION_MAX_AGE_S * 1000;
    let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    let session_id =
        queries::create_session(&state.pool, user.id, &nonce, expires_at, user_agent, None)
            .await
            .map_err(ApiError::Internal)?;

    let value = cookie::build_value(session_id, &nonce, &state.auth.session_secret);
    Ok(cookie::set_cookie_header(
        &value,
        SESSION_MAX_AGE_S,
        state.auth.cookie_secure,
    ))
}

fn authed_response(
    status: StatusCode,
    user: User,
    cookie_header: String,
    _state: &AppState,
) -> axum::response::Response {
    let mut response = (status, Json(AuthResponse { user })).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie_header).expect("ascii cookie"),
    );
    response
}
