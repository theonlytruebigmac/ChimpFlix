//! `/admin/intro-fingerprints` — operator-side listing + delete for
//! the per-show audio fingerprints that drive the intro detector.
//!
//! Two endpoints:
//!
//!   * `GET /admin/intro-fingerprints` — every captured fingerprint
//!     joined to its show title, sorted most-recent-first. Drives the
//!     Maintenance → Intro fingerprints admin page.
//!   * `DELETE /admin/intro-fingerprints/{show_id}` — wipe every
//!     fingerprint row attached to the show. Next detect-markers run
//!     falls back to blackdetect until a new capture seeds the show
//!     again.
//!
//! Both are Owner-gated — the per-show fingerprint is a system-wide
//! signature the player relies on, and a typo'd "clear all" by a
//! lower-tier admin would force every operator to re-capture across
//! the library.

use axum::Json;
use axum::extract::{Path, State};
use chimpflix_library::queries;
use serde::Serialize;
use tracing::info;

use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub fingerprints: Vec<queries::ShowIntroFingerprintListing>,
}

pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<ListResponse>, ApiError> {
    let fingerprints = queries::list_all_show_intro_fingerprints(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(ListResponse { fingerprints }))
}

#[derive(Debug, Serialize)]
pub struct DeleteResponse {
    pub removed: u64,
}

pub async fn delete_for_show(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(show_id): Path<i64>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let removed = queries::delete_all_show_intro_fingerprints(&state.pool, show_id)
        .await
        .map_err(ApiError::Internal)?;
    info!(
        show_id,
        removed,
        "show intro fingerprint(s) cleared via admin listing",
    );
    Ok(Json(DeleteResponse { removed }))
}
