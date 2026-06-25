//! Bearer-token authentication.
//!
//! `AuthedMachine` is an axum extractor: any handler that takes it requires a
//! valid `Authorization: Bearer <token>` header. The token is resolved to a
//! machine id via the configured token map; a missing/unknown token yields 401.

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::error::HubError;
use crate::state::AppState;

/// The authenticated machine id, extracted from the bearer token.
pub struct AuthedMachine(pub Uuid);

impl FromRequestParts<AppState> for AuthedMachine {
    type Rejection = HubError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or(HubError::Unauthorized)?;

        let machine_id = state.tokens.get(token).ok_or(HubError::Unauthorized)?;
        Ok(AuthedMachine(*machine_id))
    }
}
