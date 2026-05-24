//! `GET /metrics` — Prometheus exposition format.
//!
//! Operators point Prometheus / VictoriaMetrics / similar at this
//! endpoint to graph the live state of the server: pool pressure,
//! transcode session counts, queued jobs, backup retention, WAL
//! growth.
//!
//! **Auth:** unauthenticated by design (Prometheus scrapes don't
//! usually carry creds). Operators are expected to gate access at
//! the reverse proxy layer — either bind the metrics scrape route
//! to an internal IP range, or basic-auth it via Caddy/nginx. The
//! body never leaks secrets (no token / password / session content),
//! but counts + paths can still be operationally sensitive.
//!
//! **Format:** plain text in Prometheus exposition format. We hand-
//! roll instead of pulling the `prometheus` crate because the metric
//! surface here is small and the crate's macro hell isn't worth the
//! transitive deps for a v0.1 audience.
//!
//! See WEEK 1 #10 in `docs/PUBLIC_RELEASE_HARDENING.md`.

use std::fmt::Write as _;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime};

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sqlx::Row;

use crate::state::AppState;

static START: OnceLock<Instant> = OnceLock::new();

pub async fn metrics(State(state): State<AppState>) -> Response {
    let mut out = String::with_capacity(2048);

    let started = *START.get_or_init(Instant::now);
    let _ = writeln!(
        out,
        "# HELP chimpflix_uptime_seconds Process uptime."
    );
    let _ = writeln!(out, "# TYPE chimpflix_uptime_seconds gauge");
    let uptime = started.elapsed().as_secs_f64();
    let _ = writeln!(out, "chimpflix_uptime_seconds {uptime}");

    // ── DB pool ──
    let size = state.pool.size();
    let idle = state.pool.num_idle();
    let _ = writeln!(out, "# HELP chimpflix_db_pool Pool connections by state.");
    let _ = writeln!(out, "# TYPE chimpflix_db_pool gauge");
    let _ = writeln!(out, "chimpflix_db_pool{{state=\"size\"}} {size}");
    let _ = writeln!(out, "chimpflix_db_pool{{state=\"idle\"}} {idle}");

    // ── Active transcode sessions ──
    let sessions = state.transcoder.list_sessions().len();
    let _ = writeln!(
        out,
        "# HELP chimpflix_active_sessions Active HLS transcode sessions."
    );
    let _ = writeln!(out, "# TYPE chimpflix_active_sessions gauge");
    let _ = writeln!(out, "chimpflix_active_sessions {sessions}");

    // ── Jobs by kind × status ──
    if let Ok(rows) = sqlx::query("SELECT kind, status, COUNT(*) AS n FROM jobs GROUP BY kind, status")
        .fetch_all(&state.pool)
        .await
    {
        let _ = writeln!(
            out,
            "# HELP chimpflix_jobs Job-queue row counts by kind and status."
        );
        let _ = writeln!(out, "# TYPE chimpflix_jobs gauge");
        for row in rows {
            let kind: String = row.try_get("kind").unwrap_or_default();
            let status: String = row.try_get("status").unwrap_or_default();
            let n: i64 = row.try_get("n").unwrap_or(0);
            let kind_esc = escape_label(&kind);
            let status_esc = escape_label(&status);
            let _ = writeln!(
                out,
                "chimpflix_jobs{{kind=\"{kind_esc}\",status=\"{status_esc}\"}} {n}",
            );
        }
    }

    // ── Backup retention ──
    let backups_dir = state
        .data_dir
        .join(crate::api::admin::backup::AUTO_BACKUP_SUBDIR);
    let (backup_count, backup_bytes, backup_oldest_age_s) =
        scan_backups(&backups_dir).await.unwrap_or((0, 0, 0));
    let _ = writeln!(out, "# HELP chimpflix_backups Auto-backup snapshots on disk.");
    let _ = writeln!(out, "# TYPE chimpflix_backups gauge");
    let _ = writeln!(out, "chimpflix_backups{{stat=\"count\"}} {backup_count}");
    let _ = writeln!(out, "chimpflix_backups{{stat=\"bytes\"}} {backup_bytes}");
    let _ = writeln!(
        out,
        "chimpflix_backups{{stat=\"oldest_age_seconds\"}} {backup_oldest_age_s}",
    );

    // ── SQLite WAL size ──
    let wal_path = state.data_dir.join("chimpflix.db-wal");
    let wal_bytes: u64 = tokio::fs::metadata(&wal_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let _ = writeln!(
        out,
        "# HELP chimpflix_sqlite_wal_bytes SQLite WAL file size in bytes."
    );
    let _ = writeln!(out, "# TYPE chimpflix_sqlite_wal_bytes gauge");
    let _ = writeln!(out, "chimpflix_sqlite_wal_bytes {wal_bytes}");

    // ── HTTP request counters + cumulative latency ──
    let http = state.http_metrics.snapshot().await;
    if !http.is_empty() {
        let _ = writeln!(
            out,
            "# HELP chimpflix_http_requests_total Total HTTP requests by route + method + status class."
        );
        let _ = writeln!(out, "# TYPE chimpflix_http_requests_total counter");
        for (key, count, _) in &http {
            let route = escape_label(&key.route);
            let method = escape_label(&key.method);
            let _ = writeln!(
                out,
                "chimpflix_http_requests_total{{route=\"{route}\",method=\"{method}\",status=\"{}\"}} {count}",
                key.status_class,
            );
        }
        let _ = writeln!(
            out,
            "# HELP chimpflix_http_request_duration_seconds_sum Cumulative request duration in seconds."
        );
        let _ = writeln!(
            out,
            "# TYPE chimpflix_http_request_duration_seconds_sum counter"
        );
        for (key, _, duration_us) in &http {
            let route = escape_label(&key.route);
            let method = escape_label(&key.method);
            // Convert microseconds → seconds for Prometheus convention.
            let seconds = *duration_us as f64 / 1_000_000.0;
            let _ = writeln!(
                out,
                "chimpflix_http_request_duration_seconds_sum{{route=\"{route}\",method=\"{method}\",status=\"{}\"}} {seconds}",
                key.status_class,
            );
        }
    }

    let mut resp = (StatusCode::OK, out).into_response();
    // Use the actual content type Prometheus expects so its scrape
    // parser doesn't fall back to autodetection.
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4"),
    );
    resp
}

async fn scan_backups(dir: &std::path::Path) -> std::io::Result<(usize, u64, u64)> {
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0, 0)),
        Err(e) => return Err(e),
    };
    let mut count = 0usize;
    let mut bytes = 0u64;
    let mut oldest = SystemTime::now();
    let mut found_any = false;
    while let Some(ent) = rd.next_entry().await? {
        let path = ent.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !(name.starts_with("chimpflix-") && name.ends_with(".db")) {
            continue;
        }
        let meta = ent.metadata().await?;
        if !meta.is_file() {
            continue;
        }
        count += 1;
        bytes += meta.len();
        if let Ok(mtime) = meta.modified() {
            if mtime < oldest {
                oldest = mtime;
                found_any = true;
            }
        }
    }
    let oldest_age_s = if found_any {
        SystemTime::now()
            .duration_since(oldest)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    } else {
        0
    };
    Ok((count, bytes, oldest_age_s))
}

/// Escape a label value per Prometheus exposition rules:
/// backslash, double-quote, and newline get escaped. Most of our
/// labels are safe identifiers (job kinds + statuses) but the
/// escape keeps us honest if a new kind ever contains punctuation.
fn escape_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str(r#"\""#),
            '\n' => out.push_str(r"\n"),
            _ => out.push(c),
        }
    }
    out
}
