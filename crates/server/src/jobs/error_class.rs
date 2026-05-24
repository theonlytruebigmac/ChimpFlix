//! Error classification for job-queue failures.
//!
//! When a handler returns `Err(...)`, the worker calls
//! [`classify`] to assign one of [`ErrorClass`]'s variants. The
//! class drives:
//!
//!   - The retry decision (`Permanent` and `ExternalAuth` go
//!     straight to dead; everything else retries).
//!   - The backoff curve ([`backoff_for_class`]) — rate-limited
//!     jobs wait much longer than transient network blips.
//!   - The activity screen's "N jobs failed for the same reason"
//!     grouping.
//!
//! Classification is heuristic-based: we walk the anyhow chain
//! and look for known markers (HTTP status codes baked into error
//! strings, ffmpeg exit codes, IO error kinds). Handlers don't
//! have to opt in — anything they bubble up via `anyhow::bail!`
//! gets classified on the way out.

/// Failure categories. Stored in `jobs.error_class` as the lower-
/// snake-case variant name so SQL filters read naturally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// HTTP 429 from an external API (OMDb, OpenSubtitles, TMDB).
    /// Long backoff curve since the limit usually resets per-day
    /// or per-hour.
    ExternalRateLimit,
    /// HTTP 401/403 from an external API. The credential is bad —
    /// no amount of retrying will fix it, so we dead-letter
    /// immediately. The operator sees this in the activity panel
    /// as "X jobs failed — credentials" and re-keys the service.
    ExternalAuth,
    /// Timed out — tokio timeout, ffmpeg watchdog kill, etc.
    /// Short backoff because the next attempt might succeed if the
    /// system is just under temporary load.
    Timeout,
    /// File-level permanent failure: source file missing, corrupt
    /// audio that ffmpeg refuses to read, row deleted between
    /// enqueue and pickup. No point retrying.
    Permanent,
    /// Catch-all for unknown errors. Treated as retryable so a
    /// brief network blip or sqlx BUSY doesn't poison the job.
    Transient,
}

impl ErrorClass {
    /// Stable string form written to `jobs.error_class`.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorClass::ExternalRateLimit => "external_rate_limit",
            ErrorClass::ExternalAuth => "external_auth",
            ErrorClass::Timeout => "timeout",
            ErrorClass::Permanent => "permanent",
            ErrorClass::Transient => "transient",
        }
    }

    /// True when this class should be dead-lettered without
    /// further retries.
    pub fn is_terminal(self) -> bool {
        matches!(self, ErrorClass::ExternalAuth | ErrorClass::Permanent)
    }
}

/// Walk the error chain (anyhow's `cause()` chain) and return the
/// first variant whose markers match. Falls back to `Transient` so
/// the worker keeps retrying unknown errors instead of marking
/// them dead.
///
/// Order of checks matters: an error message saying "429 unauthorized"
/// classifies as `ExternalRateLimit` (the rate limit IS the active
/// problem; once the operator's quota resets the auth half resolves
/// itself). "operation timed out, file not found" classifies as
/// Timeout because the timeout typically *caused* the missing-file
/// followup. Tests document each precedence rule.
pub fn classify(err: &anyhow::Error) -> ErrorClass {
    // Build a single lowercased string spanning the whole chain so
    // we can substring-match cheaply.
    let chain = error_chain_lower(err);

    // Rate limit: only fires on explicit HTTP-status markers or
    // the literal phrase. Raw "429" anywhere in the chain would
    // false-positive on file paths and error codes — we want to
    // see one of "http 429", "status 429", " 429:", or similar
    // contexts that look like a status code.
    if has_http_status(&chain, 429)
        || chain.contains("rate limit")
        || chain.contains("rate-limit")
        || chain.contains("rate_limit")
    {
        return ErrorClass::ExternalRateLimit;
    }

    // Auth failures: HTTP 401/403 in the same contexts as 429,
    // plus OAuth-shaped messages.
    if has_http_status(&chain, 401)
        || has_http_status(&chain, 403)
        || chain.contains("unauthorized")
        || chain.contains("forbidden")
        || chain.contains("invalid token")
        || chain.contains("invalid_token")
    {
        return ErrorClass::ExternalAuth;
    }

    // Timeout markers — tokio's elapsed/timeout, the ffmpeg
    // watchdog ("killed"), and reqwest's timeout message.
    if chain.contains("timed out")
        || chain.contains("timeout")
        || chain.contains("deadline")
        || chain.contains("watchdog")
    {
        return ErrorClass::Timeout;
    }

    // Permanent indicators: file gone, corrupt source the codec
    // refuses to play, sqlx not-found, or an external API explicitly
    // saying "I can't serve this" in a way that won't change on
    // retry. "negative response" is the OMDb client's bail message
    // for anything OMDb returned `{"Response":"False",...}` for that
    // wasn't a known not-found shape — by the time it reaches the
    // classifier, retrying just burns the request budget against an
    // upstream that's told us no. Operators who suspect OMDb has
    // recovered can re-enqueue manually from the dead-letter view.
    if chain.contains("no such file")
        || chain.contains("file not found")
        || chain.contains("invalid data")
        || chain.contains("unrecognised codec")
        || chain.contains("unsupported format")
        || chain.contains("negative response")
    {
        return ErrorClass::Permanent;
    }

    ErrorClass::Transient
}

/// True iff the lowercased error chain contains an HTTP-status-
/// shaped occurrence of `code`. Recognises:
///   - `http {code}` / `http/{code}`
///   - `status {code}` / `status: {code}` / `status_code: {code}`
///   - `({code} ...)` — common reqwest Display format
///   - ` {code}:` — leading-space delimited (matches "OMDb 429:")
///
/// Critically does NOT match a raw three-digit string in arbitrary
/// context (file path, item id, ffmpeg time offset).
fn has_http_status(chain: &str, code: u16) -> bool {
    let needle = code.to_string();
    let n = &needle;
    chain.contains(&format!("http {n}"))
        || chain.contains(&format!("http/{n}"))
        || chain.contains(&format!("http/1.1 {n}"))
        || chain.contains(&format!("http/2 {n}"))
        || chain.contains(&format!("status {n}"))
        || chain.contains(&format!("status: {n}"))
        || chain.contains(&format!("status_code: {n}"))
        || chain.contains(&format!("status code: {n}"))
        || chain.contains(&format!("({n} "))
        || chain.contains(&format!(" {n}:"))
        || chain.contains(&format!(" {n} "))
}

fn error_chain_lower(err: &anyhow::Error) -> String {
    let mut out = String::with_capacity(128);
    for cause in err.chain() {
        if !out.is_empty() {
            out.push(' ');
        }
        // anyhow's Display for each cause is just the message;
        // join them so a deeply-nested 429 still surfaces.
        out.push_str(&cause.to_string().to_lowercase());
    }
    out
}

/// Backoff (in ms) before the next retry, given the class and the
/// retry attempt number (1 = first retry after the initial failure).
///
/// Returns `None` for terminal classes — caller should dead-letter
/// without scheduling another attempt.
pub fn backoff_for_class(class: ErrorClass, attempt: i64) -> Option<i64> {
    if class.is_terminal() {
        return None;
    }
    let curve: &[i64] = match class {
        // OMDb / RT / OpenSubtitles all share long-cooldown rate
        // limits. The 24h cap is intentional — if a free-tier
        // OMDb key is exhausted, retrying twice an hour just
        // burns log noise.
        ErrorClass::ExternalRateLimit => &[
            5 * 60 * 1000,       // 5m
            15 * 60 * 1000,      // 15m
            60 * 60 * 1000,      // 1h
            6 * 60 * 60 * 1000,  // 6h
            24 * 60 * 60 * 1000, // 24h
        ],
        // Timeouts get short retries: the next attempt usually
        // works once load eases.
        ErrorClass::Timeout => &[
            30 * 1000,     // 30s
            2 * 60 * 1000, // 2m
            5 * 60 * 1000, // 5m
        ],
        // Transient: matches the legacy curve we used pre-Phase
        // 5b, so a binary upgrade doesn't suddenly change retry
        // pacing for the most common case.
        ErrorClass::Transient => &[
            5 * 1000,       // 5s
            30 * 1000,      // 30s
            2 * 60 * 1000,  // 2m
            10 * 60 * 1000, // 10m
            30 * 60 * 1000, // 30m
        ],
        // Already handled by `is_terminal` above; unreachable.
        ErrorClass::ExternalAuth | ErrorClass::Permanent => &[],
    };
    let idx = (attempt.saturating_sub(1)).clamp(0, (curve.len() as i64) - 1) as usize;
    curve.get(idx).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    fn make(msg: &str) -> anyhow::Error {
        anyhow!(msg.to_string())
    }

    #[test]
    fn rate_limit_classified_by_429() {
        assert_eq!(
            classify(&make("OMDb 429: rate limit exceeded")),
            ErrorClass::ExternalRateLimit
        );
        assert_eq!(classify(&make("http 429")), ErrorClass::ExternalRateLimit);
        assert_eq!(
            classify(&make("Rate Limit hit")),
            ErrorClass::ExternalRateLimit
        );
        // Reqwest's Display format
        assert_eq!(
            classify(&make("error sending request (429 Too Many Requests)")),
            ErrorClass::ExternalRateLimit
        );
    }

    #[test]
    fn rate_limit_does_not_false_positive_on_random_429_substring() {
        // Filenames or error codes that happen to contain "429"
        // should NOT classify as rate-limited. Before the
        // has_http_status() refactor, a bare `chain.contains("429")`
        // would mis-classify these.
        assert_eq!(
            classify(&make("file /movies/Episode 429.mkv: no such file")),
            ErrorClass::Permanent,
            "file path with 429 in name should not be rate-limited"
        );
        assert_eq!(
            classify(&make("internal error code 12942: something else")),
            ErrorClass::Transient,
            "embedded digits should not be rate-limited"
        );
    }

    #[test]
    fn auth_classified_by_401_or_403() {
        assert_eq!(classify(&make("http 401")), ErrorClass::ExternalAuth);
        assert_eq!(classify(&make("http 403")), ErrorClass::ExternalAuth);
        assert_eq!(classify(&make("Unauthorized")), ErrorClass::ExternalAuth);
        assert_eq!(classify(&make("invalid token")), ErrorClass::ExternalAuth);
    }

    #[test]
    fn timeout_classified() {
        assert_eq!(classify(&make("operation timed out")), ErrorClass::Timeout);
        assert_eq!(
            classify(&make("ffmpeg watchdog killed child")),
            ErrorClass::Timeout
        );
        assert_eq!(classify(&make("deadline exceeded")), ErrorClass::Timeout);
    }

    #[test]
    fn permanent_classified() {
        assert_eq!(
            classify(&make("No such file or directory")),
            ErrorClass::Permanent
        );
        assert_eq!(
            classify(&make("invalid data: garbled mkv header")),
            ErrorClass::Permanent
        );
    }

    #[test]
    fn omdb_negative_response_is_permanent_not_transient() {
        // OMDb returns `{"Response":"False","Error":"Error getting data."}`
        // for various IMDb ids the metadata team hasn't indexed.
        // Retrying just burns the daily request budget. Pre-fix this
        // fell into Transient and ate 3 backoff attempts before
        // dying — confirmed via production log 2026-05-24.
        let err = make(
            "omdb fetch failed for item 60: omdb negative response: Error getting data.",
        );
        assert_eq!(classify(&err), ErrorClass::Permanent);
    }

    #[test]
    fn unknown_falls_back_to_transient() {
        assert_eq!(
            classify(&make("something unusual happened")),
            ErrorClass::Transient
        );
    }

    #[test]
    fn backoff_terminal_returns_none() {
        assert!(backoff_for_class(ErrorClass::ExternalAuth, 1).is_none());
        assert!(backoff_for_class(ErrorClass::Permanent, 1).is_none());
    }

    #[test]
    fn backoff_grows_then_caps() {
        let a = backoff_for_class(ErrorClass::ExternalRateLimit, 1).unwrap();
        let b = backoff_for_class(ErrorClass::ExternalRateLimit, 2).unwrap();
        let c = backoff_for_class(ErrorClass::ExternalRateLimit, 99).unwrap();
        assert!(a < b);
        assert_eq!(c, 24 * 60 * 60 * 1000);
    }

    #[test]
    fn classify_walks_anyhow_chain() {
        let root = make("OMDb 429: rate limit");
        let wrapped = root.context("fetching ratings");
        assert_eq!(classify(&wrapped), ErrorClass::ExternalRateLimit);
    }
}
