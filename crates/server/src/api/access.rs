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
//!
//! BROWSE vs PLAYBACK (phase 107 tri-state access). The `ensure_*_accessible`
//! helpers below are the BROWSE gate: they admit anyone whose effective
//! level is `view` OR `full` (they ride on `user_library_filter`, which
//! unions every grant row regardless of level). Starting a stream/transcode
//! is a STRICTER gate — it requires the effective level to be `full`. That
//! lives in [`ensure_file_playable`], which resolves the file's owning
//! library and rejects with `Forbidden` when the level is only `view`.

use chimpflix_library::AccessLevel;
use chimpflix_library::queries;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

/// Reject the request if the user can't BROWSE the library this *media
/// file* belongs to (effective level `view` OR `full`). Returns NotFound
/// (not Forbidden) so existence isn't leaked.
///
/// Phase 107: the only file-id route today is direct play, which uses the
/// stricter [`ensure_file_playable`] gate, so this browse-level helper has
/// no current caller — it's retained alongside the item/episode/subtitle
/// browse gates for any future file-metadata-only endpoint that should be
/// reachable by `view` users.
#[allow(dead_code)]
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

/// PLAYBACK gate. Reject the request unless the user's EFFECTIVE access
/// level for the library owning this *media file* is `full`. A `view`-only
/// user can browse the library + item metadata but gets `Forbidden` here
/// when they try to start a stream/transcode. A user with no access at all
/// gets `NotFound` (existence isn't leaked, matching the browse helpers).
///
/// This is the single playback authorization funnel used by every
/// stream-start path (direct play, transcode/HLS session create, prewarm).
/// Owners always resolve to `full`, so they're never gated.
pub async fn ensure_file_playable(
    state: &AppState,
    user: &AuthUser,
    file_id: i64,
) -> Result<(), ApiError> {
    let lib_id = queries::media_file_library_id(&state.pool, file_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    let level = queries::user_effective_access_level(&state.pool, user.id, user.role, lib_id)
        .await
        .map_err(ApiError::Internal)?;
    match level {
        // No grant at all — don't leak existence; behave like the file
        // isn't there (same as the browse helpers).
        AccessLevel::None => Err(ApiError::NotFound),
        // Browse-only — the library + item are visible, so 403 (not 404)
        // is honest: "you can see it but can't play it".
        AccessLevel::View => Err(ApiError::Forbidden),
        AccessLevel::Full => Ok(()),
    }
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
