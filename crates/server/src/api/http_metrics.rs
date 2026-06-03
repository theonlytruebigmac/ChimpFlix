//! Per-route HTTP request / latency counters.
//!
//! Records a count + cumulative-microsecond total per (route, method,
//! status_class) tuple. The Prometheus exporter ([`super::metrics`])
//! reads these and emits Prometheus counter / histogram-summary lines.
//!
//! Route key uses `axum::extract::MatchedPath` so `/items/42` and
//! `/items/99` aggregate under `/api/v1/items/{id}` rather than
//! exploding to one bucket per URL. Falls back to the literal path
//! when no MatchedPath is present (rare; mostly non-axum-routed
//! requests like fallback 404s).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::extract::MatchedPath;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use tokio::sync::RwLock;

/// Live counters for one (route, method, status_class) bucket.
#[derive(Default)]
pub struct RouteCounter {
    pub count: AtomicU64,
    pub duration_us: AtomicU64,
}

/// All-route registry. `Arc` so cheap to clone between the middleware
/// and the metrics exporter.
#[derive(Clone, Default)]
pub struct HttpMetricsRegistry {
    inner: Arc<RwLock<HashMap<MetricKey, Arc<RouteCounter>>>>,
}

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct MetricKey {
    pub route: String,
    pub method: String,
    /// `2xx` / `3xx` / `4xx` / `5xx` — collapsed so the cardinality
    /// stays bounded.
    pub status_class: &'static str,
}

impl HttpMetricsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn snapshot(&self) -> Vec<(MetricKey, u64, u64)> {
        let guard = self.inner.read().await;
        guard
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.count.load(Ordering::Relaxed),
                    v.duration_us.load(Ordering::Relaxed),
                )
            })
            .collect()
    }

    async fn record(&self, key: MetricKey, micros: u64) {
        // Fast path: read lock + atomic increment.
        if let Some(entry) = self.inner.read().await.get(&key) {
            entry.count.fetch_add(1, Ordering::Relaxed);
            entry.duration_us.fetch_add(micros, Ordering::Relaxed);
            return;
        }
        // Slow path: insert + record.
        let mut guard = self.inner.write().await;
        let entry = guard.entry(key).or_default();
        entry.count.fetch_add(1, Ordering::Relaxed);
        entry.duration_us.fetch_add(micros, Ordering::Relaxed);
    }
}

/// axum middleware. Wire on the outer Router so every route — auth /
/// API / metrics / health — gets counted; metrics endpoints land in
/// their own bucket and contribute to the total.
pub async fn track(
    axum::extract::State(registry): axum::extract::State<HttpMetricsRegistry>,
    req: Request,
    next: Next,
) -> Response {
    let start = Instant::now();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        // Collapse all unmatched paths (bot scans, stray 404s) into one
        // sentinel bucket so the HashMap cardinality stays bounded.
        .unwrap_or_else(|| "<unmatched>".to_string());
    let method = req.method().to_string();
    let response = next.run(req).await;
    let status = response.status();
    let status_class: &'static str = match status.as_u16() / 100 {
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        5 => "5xx",
        _ => "other",
    };
    let micros = start.elapsed().as_micros() as u64;
    registry
        .record(
            MetricKey {
                route,
                method,
                status_class,
            },
            micros,
        )
        .await;
    response
}
