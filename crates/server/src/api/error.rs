//! Unified API error type with JSON `IntoResponse`.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use thiserror::Error;
use tracing::error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
    /// Like NotFound but with a short resource label (e.g. "backup",
    /// "library", "item") so operators reading logs / API responses
    /// can tell what wasn't found instead of seeing a bare "not found".
    /// New code should prefer this over `NotFound`.
    #[error("{0} not found")]
    NotFoundResource(&'static str),
    #[error("{0}")]
    Validation(String),
    #[error("unauthenticated")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("too many requests: {0}")]
    TooManyRequests(String),
    /// Upstream third-party API (TMDB, TVDB, OpenSubtitles, …) returned
    /// something we can't act on — empty body, malformed JSON, 5xx, etc.
    /// Distinct from Internal so the UI can render "try again" instead
    /// of a generic 500 banner.
    #[error("bad upstream: {0}")]
    BadGateway(String),
    /// 507 Insufficient Storage — used by the backup endpoint when
    /// the partition can't fit a snapshot. See MONTH 1 in
    /// `docs/PUBLIC_RELEASE_HARDENING.md`.
    #[error("insufficient storage: {0}")]
    InsufficientStorage(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl ApiError {
    pub fn validation(msg: impl Into<String>) -> Self {
        ApiError::Validation(msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // Pull a Retry-After hint out of TooManyRequests messages of the
        // form "...try again in {N}s". Lets login lockouts (handed to us
        // as ApiError::TooManyRequests by the per-identity attempt
        // tracker) emit the same Retry-After header the IP-rate-limit
        // middleware sets, so clients and ops tooling get a consistent
        // contract regardless of which path triggered the 429.
        let retry_after_s = if let ApiError::TooManyRequests(m) = &self {
            extract_retry_after_secs(m)
        } else {
            None
        };
        let (status, code, message): (StatusCode, &'static str, String) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found", "not found".to_string()),
            ApiError::NotFoundResource(label) => (
                StatusCode::NOT_FOUND,
                "not_found",
                format!("{label} not found"),
            ),
            ApiError::Validation(m) => (StatusCode::BAD_REQUEST, "validation_failed", m.clone()),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "authentication required".to_string(),
            ),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "forbidden".to_string()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone()),
            ApiError::TooManyRequests(m) => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                m.clone(),
            ),
            ApiError::BadGateway(m) => (StatusCode::BAD_GATEWAY, "bad_upstream", m.clone()),
            ApiError::InsufficientStorage(m) => (
                StatusCode::INSUFFICIENT_STORAGE,
                "insufficient_storage",
                m.clone(),
            ),
            ApiError::Internal(e) => {
                error!(error = %format!("{e:#}"), "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error".to_string(),
                )
            }
        };
        let body = Json(json!({
            "error": { "code": code, "message": message }
        }));
        let mut resp = (status, body).into_response();
        if let Some(secs) = retry_after_s {
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                resp.headers_mut().insert(header::RETRY_AFTER, v);
            }
        }
        resp
    }
}

/// Parse "...try again in {N}s" suffix out of a TooManyRequests message
/// so the IntoResponse impl can echo it as a Retry-After header. Returns
/// None for any other shape; the header is then omitted.
fn extract_retry_after_secs(message: &str) -> Option<u64> {
    let tail = message.rsplit_once("try again in ").map(|(_, t)| t)?;
    let num = tail.split('s').next()?.trim();
    num.parse::<u64>().ok()
}
