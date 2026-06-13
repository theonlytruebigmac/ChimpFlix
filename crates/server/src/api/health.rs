//! Health and server-info endpoints.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chimpflix_library::queries;
use serde::Serialize;
use sqlx::{Executor, Row};

use crate::api::error::ApiError;
use crate::state::AppState;

/// Cached result of the last ffmpeg `-version` probe and the time it was
/// recorded. The outer `OnceLock` initialises lazily on the first `/ready`
/// call; after that a fresh probe is only issued when the cached value is
/// older than `FFMPEG_CACHE_TTL`. This prevents every unauthenticated
/// `/ready` request from forking a new process.
static FFMPEG_CACHE: OnceLock<Mutex<(CheckStatus, Instant)>> = OnceLock::new();

/// Re-probe ffmpeg at most once per this interval.
const FFMPEG_CACHE_TTL: Duration = Duration::from_secs(30);

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
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let uptime_s = ((chimpflix_common::now_ms() - state.started_at_ms) / 1000).max(0) as u64;
    Json(HealthResponse {
        status: "ok",
        uptime_s,
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

#[derive(Debug, Clone, Serialize)]
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
    let uptime_s = ((chimpflix_common::now_ms() - state.started_at_ms) / 1000).max(0) as u64;
    let body = ReadyResponse {
        status: if any_failed { "failed" } else { "ok" },
        uptime_s,
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
    // Opaque detail: raw SQLite error text is not surfaced to unauthenticated
    // callers — it can reveal schema names, file paths, or lock state.
    match state.pool.execute("SELECT 1").await {
        Ok(_) => CheckStatus::Ok,
        Err(_) => CheckStatus::Failed {
            detail: "database ping failed".to_string(),
        },
    }
}

async fn check_ffmpeg(state: &AppState) -> CheckStatus {
    // Check whether the cached result is still fresh (within TTL).
    let cache = FFMPEG_CACHE.get_or_init(|| {
        // Initialise with a zero-age failed sentinel so the first call
        // always runs the real probe.
        Mutex::new((
            CheckStatus::Failed {
                detail: "not yet probed".to_string(),
            },
            Instant::now() - FFMPEG_CACHE_TTL - Duration::from_secs(1),
        ))
    });

    {
        let guard = cache.lock().unwrap_or_else(|p| p.into_inner());
        if guard.1.elapsed() < FFMPEG_CACHE_TTL {
            return guard.0.clone();
        }
    }

    // Cache is stale — run the real probe and store the result.
    let bin = state.ffmpeg.ffmpeg.clone();
    let result = tokio::process::Command::new(&bin)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    // Opaque detail: do not include the binary path or OS error text in
    // the unauthenticated response — those strings are visible to any
    // caller and aid reconnaissance.
    let status = match result {
        Ok(out) if out.status.success() => CheckStatus::Ok,
        Ok(_) => CheckStatus::Failed {
            detail: "ffmpeg exited with a non-zero status".to_string(),
        },
        Err(_) => CheckStatus::Failed {
            detail: "could not probe ffmpeg".to_string(),
        },
    };

    let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());
    *guard = (status.clone(), Instant::now());
    status
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
        // Opaque: do not forward raw SQLite errors to unauthenticated callers.
        Err(_) => {
            return CheckStatus::Failed {
                detail: "could not query library paths".to_string(),
            };
        }
    };
    if rows.is_empty() {
        return CheckStatus::Degraded {
            detail: "no library paths configured yet".to_string(),
        };
    }
    let mut missing_count: usize = 0;
    for row in &rows {
        let path: String = match row.try_get("path") {
            Ok(p) => p,
            Err(_) => continue,
        };
        // `tokio::fs::metadata` follows symlinks (the operator may
        // have mounted under `/mnt` and symlinked into `/data`). A
        // missing or unreadable target counts as failed.
        // Opaque count only — filesystem paths and OS error text are
        // not included in the unauthenticated response.
        match tokio::fs::metadata(&path).await {
            Ok(m) if m.is_dir() => {}
            _ => missing_count += 1,
        }
    }
    if missing_count == 0 {
        CheckStatus::Ok
    } else {
        CheckStatus::Failed {
            detail: format!("{missing_count} library path(s) unreachable"),
        }
    }
}

async fn check_vault(state: &AppState) -> CheckStatus {
    // Opaque details: vault row IDs, decryption error messages, and
    // query errors must not be forwarded to unauthenticated callers.
    match queries::vault_self_test(&state.pool, &state.vault).await {
        Ok(queries::VaultSelfTest::Ok { .. }) => CheckStatus::Ok,
        Ok(queries::VaultSelfTest::NoEncryptedRows) => CheckStatus::Degraded {
            detail: "no encrypted rows yet (fresh install or pre-credentials boot)".to_string(),
        },
        Ok(queries::VaultSelfTest::Mismatch { .. }) => CheckStatus::Failed {
            detail: "vault decrypt self-test failed".to_string(),
        },
        Err(_) => CheckStatus::Failed {
            detail: "vault self-test query failed".to_string(),
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
