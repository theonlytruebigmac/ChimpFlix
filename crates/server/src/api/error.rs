//! Unified API error type with JSON `IntoResponse`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use thiserror::Error;
use tracing::error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
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
        let (status, code, message): (StatusCode, &'static str, String) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found", "not found".to_string()),
            ApiError::Validation(m) => (StatusCode::BAD_REQUEST, "validation_failed", m.clone()),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "authentication required".to_string(),
            ),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "forbidden".to_string()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone()),
            ApiError::TooManyRequests(m) => {
                (StatusCode::TOO_MANY_REQUESTS, "too_many_requests", m.clone())
            }
            ApiError::BadGateway(m) => (StatusCode::BAD_GATEWAY, "bad_upstream", m.clone()),
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
        (status, body).into_response()
    }
}
