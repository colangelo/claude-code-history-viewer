//! Hub error type and its mapping to HTTP responses.

use axum::http::StatusCode;
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
            HubError::Internal(m) => {
                tracing::error!(error = %m, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
