//! Bearer-token authentication.
//!
//! Two extractors gate handlers on a valid `Authorization: Bearer <token>`
//! header (a missing/unknown token yields 401):
//! - [`AuthedMachine`] resolves the token to a machine id — used by ingest,
//!   which writes under that machine's identity.
//! - [`Authenticated`] only proves the caller holds a valid token — used by the
//!   read endpoints, which span all machines in the archive.

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::error::HubError;
use crate::state::AppState;

/// Resolve the bearer token from request headers to its machine id, or 401.
fn resolve_machine(parts: &Parts, state: &AppState) -> Result<Uuid, HubError> {
    let token = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or(HubError::Unauthorized)?;
    state
        .tokens
        .get(token)
        .copied()
        .ok_or(HubError::Unauthorized)
}

/// The authenticated machine id, extracted from the bearer token.
pub struct AuthedMachine(pub Uuid);

impl FromRequestParts<AppState> for AuthedMachine {
    type Rejection = HubError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(AuthedMachine(resolve_machine(parts, state)?))
    }
}

/// Proof that the caller holds a valid token. Read endpoints query across all
/// machines, so they only need authentication, not a bound machine identity.
pub struct Authenticated;

impl FromRequestParts<AppState> for Authenticated {
    type Rejection = HubError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        resolve_machine(parts, state)?;
        Ok(Authenticated)
    }
}
