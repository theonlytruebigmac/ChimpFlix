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
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ScanJob>, ApiError> {
    let job = queries::get_scan_job(&state.pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(job))
}
