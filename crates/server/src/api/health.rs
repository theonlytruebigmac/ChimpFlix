//! Health and server-info endpoints.

use std::sync::OnceLock;
use std::time::Instant;

use axum::Json;
use axum::extract::State;
use chimpflix_library::queries;
use serde::Serialize;

use crate::api::error::ApiError;
use crate::state::AppState;

static START: OnceLock<Instant> = OnceLock::new();

fn started_at() -> Instant {
    *START.get_or_init(Instant::now)
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_s: u64,
}

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_s: started_at().elapsed().as_secs(),
    })
}

#[derive(Debug, Serialize)]
pub struct ServerInfoResponse {
    pub version: &'static str,
    pub library_counts: LibraryCounts,
    pub tmdb_enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct LibraryCounts {
    pub libraries: i64,
    pub movies: i64,
    pub shows: i64,
    pub episodes: i64,
}

pub async fn server_info(
    State(state): State<AppState>,
    _user: crate::auth::AuthUser,
) -> Result<Json<ServerInfoResponse>, ApiError> {
    use sqlx::Row;

    let libraries = queries::list_libraries(&state.pool).await?.len() as i64;
    let movies: i64 = sqlx::query("SELECT COUNT(*) AS n FROM items WHERE kind = 'movie'")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .try_get("n")
        .map_err(|e| ApiError::Internal(e.into()))?;
    let shows: i64 = sqlx::query("SELECT COUNT(*) AS n FROM items WHERE kind = 'show'")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .try_get("n")
        .map_err(|e| ApiError::Internal(e.into()))?;
    let episodes: i64 = sqlx::query("SELECT COUNT(*) AS n FROM episodes")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .try_get("n")
        .map_err(|e| ApiError::Internal(e.into()))?;

    Ok(Json(ServerInfoResponse {
        version: env!("CARGO_PKG_VERSION"),
        library_counts: LibraryCounts {
            libraries,
            movies,
            shows,
            episodes,
        },
        tmdb_enabled: state.tmdb.is_some(),
    }))
}
