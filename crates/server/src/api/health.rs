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
    user: crate::auth::AuthUser,
) -> Result<Json<ServerInfoResponse>, ApiError> {
    use sqlx::Row;

    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let libs = queries::list_libraries(&state.pool, acc.as_deref()).await?;
    let libraries = libs.len() as i64;
    // For non-owners, scope item/episode counts to libraries they can see.
    // Two phrasings of the same filter: bare `library_id` for queries
    // against `items`, and `i.library_id` for the JOIN against `i`.
    let render_filter = |col: &str| -> String {
        match &acc {
            None => "1=1".to_string(),
            Some(ids) if ids.is_empty() => format!("{col} = 0"),
            Some(ids) => format!(
                "{col} IN ({})",
                ids.iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        }
    };
    let item_filter = render_filter("library_id");
    let join_filter = render_filter("i.library_id");
    let movies: i64 = sqlx::query(&format!(
        "SELECT COUNT(*) AS n FROM items WHERE kind = 'movie' AND {item_filter}",
    ))
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .try_get("n")
    .map_err(|e| ApiError::Internal(e.into()))?;
    let shows: i64 = sqlx::query(&format!(
        "SELECT COUNT(*) AS n FROM items WHERE kind = 'show' AND {item_filter}",
    ))
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .try_get("n")
    .map_err(|e| ApiError::Internal(e.into()))?;
    let episodes: i64 = sqlx::query(&format!(
        "SELECT COUNT(*) AS n FROM episodes e \
         JOIN seasons s ON s.id = e.season_id \
         JOIN items i ON i.id = s.show_id \
         WHERE {join_filter}",
    ))
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
