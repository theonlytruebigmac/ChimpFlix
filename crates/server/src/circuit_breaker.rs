//! Per-client circuit breakers for external metadata providers.
//!
//! Ported from the abandoned turso-migration branch — DB-agnostic, so it
//! applies unchanged to the SQLite build. The code that calls
//! TMDB/TVDB/AniList/Trakt/OMDb/OpenSubtitles/Plex lives in background job
//! handlers and scheduled tasks. The job queue already applies class-aware
//! backoff (`ExternalRateLimit`: 5m→15m→1h→6h→24h) when a provider
//! rate-limits. The circuit breaker is the complementary fast-path: once a
//! provider has tripped the breaker, every *other* in-flight job for that
//! provider fails fast (no socket, no wait) instead of each independently
//! discovering the outage and burning a worker slot for the request
//! timeout.
//!
//! Deliberately trips ONLY on rate-limit signals (see `trips_on_rate_limit`),
//! not on benign per-item errors like a TMDB 404, so a library full of
//! unmatched titles can't wrongly open it.
//!
//! State machine: `Closed` → (N consecutive trips) → `Open` → (cooldown)
//! → `HalfOpen` (one probe allowed) → `Closed` on success / `Open` on
//! failure. State is per-process and resets on restart — acceptable for
//! a self-hosted server (a real outage re-opens within N probes).

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

impl BreakerState {
    /// Stable label for `/metrics`.
    pub fn as_str(self) -> &'static str {
        match self {
            BreakerState::Closed => "closed",
            BreakerState::Open => "open",
            BreakerState::HalfOpen => "half_open",
        }
    }
}

struct Inner {
    state: BreakerState,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
}

pub struct CircuitBreaker {
    name: &'static str,
    threshold: u32,
    open_duration: Duration,
    inner: Mutex<Inner>,
}

impl CircuitBreaker {
    pub fn new(name: &'static str, threshold: u32, open_duration: Duration) -> Self {
        Self {
            name,
            threshold: threshold.max(1),
            open_duration,
            inner: Mutex::new(Inner {
                state: BreakerState::Closed,
                consecutive_failures: 0,
                opened_at: None,
            }),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Whether a call may proceed right now. `Open` flips to `HalfOpen`
    /// (allowing exactly one probe) once the cooldown has elapsed.
    pub fn allow(&self) -> bool {
        let mut g = self.inner.lock().unwrap();
        match g.state {
            BreakerState::Closed => true,
            // A probe is already in flight; block additional concurrent callers
            // so only one request is used to test whether the provider recovered.
            BreakerState::HalfOpen => false,
            BreakerState::Open => {
                let cooled = g
                    .opened_at
                    .map(|t| t.elapsed() >= self.open_duration)
                    .unwrap_or(true);
                if cooled {
                    g.state = BreakerState::HalfOpen;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn record_success(&self) {
        let mut g = self.inner.lock().unwrap();
        g.consecutive_failures = 0;
        g.state = BreakerState::Closed;
        g.opened_at = None;
    }

    pub fn record_failure(&self) {
        let mut g = self.inner.lock().unwrap();
        g.consecutive_failures = g.consecutive_failures.saturating_add(1);
        // A failed HalfOpen probe re-opens immediately; otherwise open
        // once the consecutive-failure threshold is reached.
        if g.state == BreakerState::HalfOpen || g.consecutive_failures >= self.threshold {
            g.state = BreakerState::Open;
            g.opened_at = Some(Instant::now());
        }
    }

    pub fn state(&self) -> BreakerState {
        self.inner.lock().unwrap().state
    }

    /// Run `f` through the breaker. Bails immediately if the breaker is
    /// open. On completion, records success or — only when `is_trip`
    /// classifies the error as a circuit-worthy failure (rate limit /
    /// sustained outage) — records a trip. Benign errors (404, parse)
    /// neither open nor reset the breaker.
    pub async fn run<F, Fut, T>(
        &self,
        is_trip: fn(&anyhow::Error) -> bool,
        f: F,
    ) -> anyhow::Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        if !self.allow() {
            anyhow::bail!("circuit '{}' is open; deferring external call", self.name);
        }
        match f().await {
            Ok(v) => {
                self.record_success();
                Ok(v)
            }
            Err(e) => {
                if is_trip(&e) {
                    self.record_failure();
                }
                Err(e)
            }
        }
    }
}

/// Default trip classifier: trip only on rate-limit errors (HTTP 429 /
/// "rate limit" in the chain), the dominant sustained-failure signal.
///
/// Reuses main's existing error classifier rather than porting turso's
/// bespoke `parse_rate_limit_seconds` marker scan — main's clients don't
/// emit that marker, and `error_class::classify` already recognises 429s
/// and rate-limit phrasing robustly (and is unit-tested against the exact
/// strings AniList/OMDb/TMDB produce).
pub fn trips_on_rate_limit(e: &anyhow::Error) -> bool {
    matches!(
        crate::jobs::error_class::classify(e),
        crate::jobs::error_class::ErrorClass::ExternalRateLimit
    )
}

/// One breaker per external provider. Cheap to clone the `Arc` wrapper in
/// `AppState`. Tunables come from env at construction.
pub struct CircuitBreakers {
    pub tmdb: CircuitBreaker,
    pub tvdb: CircuitBreaker,
    pub anilist: CircuitBreaker,
    pub trakt: CircuitBreaker,
    pub omdb: CircuitBreaker,
    pub opensubtitles: CircuitBreaker,
    pub plex: CircuitBreaker,
    pub mal: CircuitBreaker,
}

impl CircuitBreakers {
    pub fn new(threshold: u32, open_duration: Duration) -> Self {
        let mk = |name| CircuitBreaker::new(name, threshold, open_duration);
        Self {
            tmdb: mk("tmdb"),
            tvdb: mk("tvdb"),
            anilist: mk("anilist"),
            trakt: mk("trakt"),
            omdb: mk("omdb"),
            opensubtitles: mk("opensubtitles"),
            plex: mk("plex"),
            mal: mk("mal"),
        }
    }

    /// `(name, state)` for every breaker — used by the `/metrics`
    /// exporter to emit `chimpflix_circuit_breaker_state{client}`.
    pub fn snapshot(&self) -> Vec<(&'static str, BreakerState)> {
        vec![
            (self.tmdb.name(), self.tmdb.state()),
            (self.tvdb.name(), self.tvdb.state()),
            (self.anilist.name(), self.anilist.state()),
            (self.trakt.name(), self.trakt.state()),
            (self.omdb.name(), self.omdb.state()),
            (self.opensubtitles.name(), self.opensubtitles.state()),
            (self.plex.name(), self.plex.state()),
            (self.mal.name(), self.mal.state()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always_trip(_: &anyhow::Error) -> bool {
        true
    }

    #[test]
    fn opens_after_threshold_consecutive_failures() {
        let b = CircuitBreaker::new("t", 3, Duration::from_secs(60));
        assert_eq!(b.state(), BreakerState::Closed);
        assert!(b.allow());
        b.record_failure();
        b.record_failure();
        assert_eq!(
            b.state(),
            BreakerState::Closed,
            "below threshold stays closed"
        );
        b.record_failure();
        assert_eq!(b.state(), BreakerState::Open, "threshold reached → open");
        assert!(!b.allow(), "open breaker blocks calls during cooldown");
    }

    #[test]
    fn success_resets_failure_count() {
        let b = CircuitBreaker::new("t", 3, Duration::from_secs(60));
        b.record_failure();
        b.record_failure();
        b.record_success();
        b.record_failure();
        b.record_failure();
        assert_eq!(
            b.state(),
            BreakerState::Closed,
            "success reset the streak, so 2 more failures don't open it"
        );
    }

    #[test]
    fn half_open_probe_failure_reopens() {
        // Zero cooldown so `allow()` immediately transitions Open→HalfOpen.
        let b = CircuitBreaker::new("t", 1, Duration::from_secs(0));
        b.record_failure(); // → Open
        assert_eq!(b.state(), BreakerState::Open);
        assert!(b.allow(), "cooldown elapsed → half-open probe allowed");
        assert_eq!(b.state(), BreakerState::HalfOpen);
        b.record_failure(); // probe failed → straight back to Open
        assert_eq!(b.state(), BreakerState::Open);
    }

    #[test]
    fn half_open_probe_success_closes() {
        let b = CircuitBreaker::new("t", 1, Duration::from_secs(0));
        b.record_failure();
        assert!(b.allow());
        assert_eq!(b.state(), BreakerState::HalfOpen);
        b.record_success();
        assert_eq!(b.state(), BreakerState::Closed);
    }

    #[tokio::test]
    async fn run_bails_fast_when_open() {
        let b = CircuitBreaker::new("t", 1, Duration::from_secs(60));
        b.record_failure(); // → Open
        let r: anyhow::Result<()> = b
            .run(always_trip, || async { panic!("must not run when open") })
            .await;
        assert!(r.is_err(), "open breaker short-circuits without running f");
    }

    #[tokio::test]
    async fn run_does_not_trip_on_benign_error() {
        let b = CircuitBreaker::new("t", 1, Duration::from_secs(60));
        let never_trip = |_: &anyhow::Error| false;
        let _: anyhow::Result<()> = b
            .run(never_trip, || async { anyhow::bail!("404 not found") })
            .await;
        assert_eq!(
            b.state(),
            BreakerState::Closed,
            "a benign (non-trip) error must not open the breaker"
        );
    }
}
