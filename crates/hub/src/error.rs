//! Hub error type and its mapping to HTTP responses.

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug)]
pub enum HubError {
    /// Missing or invalid bearer token.
    Unauthorized,
    /// Well-formed request that failed validation.
    BadRequest(String),
    /// The referenced resource does not exist.
    NotFound(String),
    /// A database error.
    Db(sqlx::Error),
    /// A failure reading the statistics mirror.
    Mirror(duckdb::Error),
    /// The statistics read model has no data yet — a cold build is running, or
    /// the mirror file could not be opened at all. Deliberately not a fallback
    /// to the old Postgres rollups (design D4): one degraded shape, with a
    /// `Retry-After` telling the caller when to come back.
    StatsWarming { retry_after_secs: u64 },
    /// Any other server-side failure.
    Internal(String),
}

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HubError::Unauthorized => write!(f, "unauthorized"),
            HubError::BadRequest(m) => write!(f, "bad request: {m}"),
            HubError::NotFound(m) => write!(f, "not found: {m}"),
            HubError::Db(e) => write!(f, "database error: {e}"),
            HubError::Mirror(e) => write!(f, "stats mirror error: {e}"),
            HubError::StatsWarming { .. } => write!(f, "statistics mirror is warming up"),
            HubError::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for HubError {}

impl From<sqlx::Error> for HubError {
    fn from(e: sqlx::Error) -> Self {
        HubError::Db(e)
    }
}

impl From<duckdb::Error> for HubError {
    fn from(e: duckdb::Error) -> Self {
        HubError::Mirror(e)
    }
}

impl IntoResponse for HubError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            HubError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            HubError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            HubError::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            HubError::Db(e) => {
                // Log the detail server-side; do not leak it to the client.
                tracing::error!(error = %e, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
            HubError::Mirror(e) => {
                tracing::error!(error = %e, "stats mirror error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
            HubError::Internal(m) => {
                tracing::error!(error = %m, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
            // The only variant that answers with a header of its own, so it
            // builds its whole response here rather than joining the shape
            // below. `Retry-After` is the point: a client that gets a bare 503
            // has to guess, and this one is genuinely transient.
            HubError::StatsWarming { retry_after_secs } => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [(header::RETRY_AFTER, retry_after_secs.to_string())],
                    Json(json!({
                        "error": "statistics mirror is still building; retry shortly",
                        "retry_after_secs": retry_after_secs,
                    })),
                )
                    .into_response();
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
