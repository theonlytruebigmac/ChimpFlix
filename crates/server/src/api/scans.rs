//! /api/v1/scans handlers.

use axum::Json;
use axum::extract::{Path, State};
use chimpflix_library::ScanJob;
use chimpflix_library::queries;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ScanJob>, ApiError> {
    let job = queries::get_scan_job(&state.pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Guard against IDOR: ensure the caller can access the library this scan belongs to.
    // Owners (user_library_filter returns None) retain unrestricted access.
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    if let Some(ref allowed) = acc {
        if !allowed.contains(&job.library_id) {
            return Err(ApiError::NotFound);
        }
    }

    Ok(Json(job))
}
