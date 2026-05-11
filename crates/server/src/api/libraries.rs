//! /api/v1/libraries handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chimpflix_library::queries;
use chimpflix_library::{Library, LibraryUpdate, NewLibrary, ScanEmitter, ScanEvent, ScanJob};
use tracing::warn;

use crate::api::error::ApiError;
use crate::auth::{AuthUser, OwnerAuth};
use crate::events::Event;
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<ListResponse>, ApiError> {
    let libraries = queries::list_libraries(&state.pool).await?;
    Ok(Json(ListResponse { libraries }))
}

pub async fn create(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Json(input): Json<NewLibrary>,
) -> Result<(StatusCode, Json<Library>), ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError::validation("name is required"));
    }
    if input.paths.is_empty() {
        return Err(ApiError::validation(
            "paths must contain at least one entry",
        ));
    }
    let lib = queries::create_library(&state.pool, input).await?;
    Ok((StatusCode::CREATED, Json(lib)))
}

pub async fn get_one(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Library>, ApiError> {
    let lib = queries::get_library(&state.pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(lib))
}

pub async fn update(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(id): Path<i64>,
    Json(update): Json<LibraryUpdate>,
) -> Result<Json<Library>, ApiError> {
    let lib = queries::update_library(&state.pool, id, update)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(lib))
}

pub async fn delete_one(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let deleted = queries::delete_library(&state.pool, id).await?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn trigger_scan(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
) -> Result<(StatusCode, Json<ScanJob>), ApiError> {
    let _library = queries::get_library(&state.pool, library_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let job = queries::create_scan_job(&state.pool, library_id).await?;
    let job_id = job.id;

    let pool = state.pool.clone();
    let ffmpeg = state.ffmpeg.clone();
    let tmdb = state.tmdb.clone();
    let hub = state.hub.clone();

    let emitter: ScanEmitter = Arc::new(move |event: ScanEvent| {
        hub.publish(Event::Scan(event));
    });

    tokio::spawn(async move {
        if let Err(e) =
            chimpflix_library::run_scan(pool, ffmpeg, tmdb, library_id, job_id, emitter).await
        {
            warn!(error = %format!("{e:#}"), library_id, job_id, "scan task ended with error");
        }
    });

    Ok((StatusCode::ACCEPTED, Json(job)))
}

pub async fn list_scans(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(library_id): Path<i64>,
) -> Result<Json<ScanListResponse>, ApiError> {
    let jobs = queries::list_scan_jobs(&state.pool, library_id, 50).await?;
    Ok(Json(ScanListResponse { scans: jobs }))
}

#[derive(serde::Serialize)]
pub struct ListResponse {
    libraries: Vec<Library>,
}

#[derive(serde::Serialize)]
pub struct ScanListResponse {
    scans: Vec<ScanJob>,
}
