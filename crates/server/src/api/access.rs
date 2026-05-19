//! Library-access helpers for per-row resources.
//!
//! Every "ID-in-the-URL" endpoint that serves library content must
//! verify the caller has access to the owning library. We return
//! `ApiError::NotFound` (not `Forbidden`) on access denial so we don't
//! leak which item/episode/file ids exist for libraries the caller
//! can't see.
//!
//! `user_library_filter` returns `None` for admins/owners (= no scoping;
//! sees everything) and `Some(Vec<library_id>)` for plain users. We use
//! that as the gate uniformly.

use chimpflix_library::queries;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

/// Reject the request if the user can't see the library this *media
/// file* belongs to. Returns NotFound (not Forbidden) so existence
/// isn't leaked.
pub async fn ensure_file_accessible(
    state: &AppState,
    user: &AuthUser,
    file_id: i64,
) -> Result<(), ApiError> {
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let Some(allowed) = acc else {
        return Ok(());
    };
    let lib_id = queries::media_file_library_id(&state.pool, file_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if !allowed.contains(&lib_id) {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// Reject the request if the user can't see the library this *item*
/// belongs to.
pub async fn ensure_item_accessible(
    state: &AppState,
    user: &AuthUser,
    item_id: i64,
) -> Result<(), ApiError> {
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let Some(allowed) = acc else {
        return Ok(());
    };
    let lib_id = queries::item_library_id(&state.pool, item_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if !allowed.contains(&lib_id) {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// Reject the request if the user can't see the library this *episode*
/// belongs to.
pub async fn ensure_episode_accessible(
    state: &AppState,
    user: &AuthUser,
    episode_id: i64,
) -> Result<(), ApiError> {
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let Some(allowed) = acc else {
        return Ok(());
    };
    let lib_id = queries::episode_library_id(&state.pool, episode_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if !allowed.contains(&lib_id) {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// Reject the request if the user can't see the library this *external
/// subtitle* belongs to.
pub async fn ensure_external_subtitle_accessible(
    state: &AppState,
    user: &AuthUser,
    sub_id: i64,
) -> Result<(), ApiError> {
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let Some(allowed) = acc else {
        return Ok(());
    };
    let lib_id = queries::external_subtitle_library_id(&state.pool, sub_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if !allowed.contains(&lib_id) {
        return Err(ApiError::NotFound);
    }
    Ok(())
}
