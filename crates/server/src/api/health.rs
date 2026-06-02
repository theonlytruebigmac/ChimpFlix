//! Health and server-info endpoints.

use std::sync::OnceLock;
use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chimpflix_library::queries;
use serde::Serialize;
use sqlx::{Executor, Row};

use crate::api::error::ApiError;
use crate::state::AppState;

static START: OnceLock<Instant> = OnceLock::new();

fn started_at() -> Instant {
    *START.get_or_init(Instant::now)
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub uptime_s: u64,
}

/// Public unauth healthcheck — Traefik / Docker / load balancers hit
/// this. Intentionally minimal: no `version` (would aid CVE targeting),
/// no build details, no DB metrics. The authenticated `/server-info`
/// endpoint surfaces the version field for the admin UI.
///
/// **This is a "process is alive" probe.** For "process is actually
/// serving real traffic," use `/ready` instead. The docker-compose
/// healthcheck points at `/ready`; upstream load balancers that
/// expect a sub-millisecond response can stay on `/health`.
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        uptime_s: started_at().elapsed().as_secs(),
    })
}

#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub status: &'static str,
    pub uptime_s: u64,
    pub checks: ReadyChecks,
}

#[derive(Debug, Serialize)]
pub struct ReadyChecks {
    pub database: CheckStatus,
    pub ffmpeg: CheckStatus,
    pub vault: CheckStatus,
    pub library_paths: CheckStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    /// Component is present but in a degraded mode — for example, the
    /// vault has no encrypted rows yet, or ffmpeg is intentionally
    /// not configured. Counts as ready (degraded != failing).
    Degraded { detail: String },
    Failed { detail: String },
}

impl CheckStatus {
    fn is_failed(&self) -> bool {
        matches!(self, CheckStatus::Failed { .. })
    }
}

/// Deep readiness probe — the response code reflects whether this
/// process can actually serve traffic right now. Returns 200 when all
/// checks are Ok/Degraded; 503 when any check is Failed. Pointed at
/// from the docker-compose healthcheck and the BLOCK #2 smoke job in
/// CI (see `docs/PUBLIC_RELEASE_HARDENING.md`).
pub async fn ready(State(state): State<AppState>) -> Response {
    let database = check_database(&state).await;
    let ffmpeg = check_ffmpeg(&state).await;
    let vault = check_vault(&state).await;
    let library_paths = check_library_paths(&state).await;

    let any_failed =
        database.is_failed() || ffmpeg.is_failed() || vault.is_failed() || library_paths.is_failed();
    let body = ReadyResponse {
        status: if any_failed { "failed" } else { "ok" },
        uptime_s: started_at().elapsed().as_secs(),
        checks: ReadyChecks {
            database,
            ffmpeg,
            vault,
            library_paths,
        },
    };
    let status = if any_failed {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (status, Json(body)).into_response()
}

async fn check_database(state: &AppState) -> CheckStatus {
    match state.pool.execute("SELECT 1").await {
        Ok(_) => CheckStatus::Ok,
        Err(e) => CheckStatus::Failed {
            detail: format!("SELECT 1 against pool failed: {e}"),
        },
    }
}

async fn check_ffmpeg(state: &AppState) -> CheckStatus {
    let bin = state.ffmpeg.ffmpeg.clone();
    let result = tokio::process::Command::new(&bin)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    match result {
        Ok(out) if out.status.success() => CheckStatus::Ok,
        Ok(out) => CheckStatus::Failed {
            detail: format!("`{bin} -version` exited with {:?}", out.status.code()),
        },
        Err(e) => CheckStatus::Failed {
            detail: format!("could not spawn `{bin} -version`: {e}"),
        },
    }
}

/// Stat every configured library path. If any one is missing or
/// unreadable (volume unmounted, NFS share dropped), mark the whole
/// check Failed so an upstream load balancer drains this node — a
/// server that can't see its media has no business serving requests.
/// A library with zero configured paths is Degraded, not Failed:
/// fresh installs sit in that state until the operator points at
/// disk during onboarding.
async fn check_library_paths(state: &AppState) -> CheckStatus {
    let rows = match sqlx::query("SELECT path FROM library_paths")
        .fetch_all(&state.pool)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return CheckStatus::Failed {
                detail: format!("could not list library_paths: {e}"),
            };
        }
    };
    if rows.is_empty() {
        return CheckStatus::Degraded {
            detail: "no library paths configured yet".to_string(),
        };
    }
    let mut missing: Vec<String> = Vec::new();
    for row in &rows {
        let path: String = match row.try_get("path") {
            Ok(p) => p,
            Err(_) => continue,
        };
        // `tokio::fs::metadata` follows symlinks (the operator may
        // have mounted under `/mnt` and symlinked into `/data`). A
        // missing or unreadable target counts as failed; the error
        // message goes back to the operator so they can fix it.
        match tokio::fs::metadata(&path).await {
            Ok(m) if m.is_dir() => {}
            Ok(_) => missing.push(format!("{path} (not a directory)")),
            Err(e) => missing.push(format!("{path} ({e})")),
        }
    }
    if missing.is_empty() {
        CheckStatus::Ok
    } else {
        CheckStatus::Failed {
            detail: format!(
                "{} library path(s) unreachable: {}",
                missing.len(),
                missing.join("; "),
            ),
        }
    }
}

async fn check_vault(state: &AppState) -> CheckStatus {
    match queries::vault_self_test(&state.pool, &state.vault).await {
        Ok(queries::VaultSelfTest::Ok { .. }) => CheckStatus::Ok,
        Ok(queries::VaultSelfTest::NoEncryptedRows) => CheckStatus::Degraded {
            detail: "no encrypted rows yet (fresh install or pre-credentials boot)".to_string(),
        },
        Ok(queries::VaultSelfTest::Mismatch { sampled, error }) => CheckStatus::Failed {
            detail: format!("sample row {sampled} did not decrypt: {error}"),
        },
        Err(e) => CheckStatus::Failed {
            detail: format!("vault self-test query failed: {e:#}"),
        },
    }
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
        // Count only downloaded episodes. Placeholder rows (no media_files,
        // materialized to complete a season for the finale flag / calendar)
        // are not content the server has, so they're excluded from this
        // public "episodes" stat.
        "SELECT COUNT(*) AS n FROM episodes e \
         JOIN seasons s ON s.id = e.season_id \
         JOIN items i ON i.id = s.show_id \
         WHERE {join_filter} \
           AND EXISTS (SELECT 1 FROM media_files mf \
                       WHERE mf.episode_id = e.id AND mf.removed_at IS NULL)",
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
        tmdb_enabled: state.tmdb.read().await.is_some(),
    }))
}
