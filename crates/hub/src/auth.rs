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

/// Header injected by Tailscale serve for tailnet clients (absent on Funnel
/// traffic). Verified by serve itself; the hub trusts it only for logins in
/// the configured allow-list.
const TAILSCALE_USER_LOGIN: &str = "tailscale-user-login";

/// True iff the request carries a `Tailscale-User-Login` header matching the
/// configured allow-list. Grants READ scope only — ingest never calls this.
fn trusted_tailscale_identity(parts: &Parts, state: &AppState) -> bool {
    if state.trusted_identities.is_empty() {
        return false;
    }
    parts
        .headers
        .get(TAILSCALE_USER_LOGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .is_some_and(|login| {
            state
                .trusted_identities
                .iter()
                .any(|t| t.eq_ignore_ascii_case(login))
        })
}

/// Proof that the caller may read. Read endpoints query across all machines,
/// so they need authentication but not a bound machine identity: a valid
/// bearer token, or (opt-in) a trusted Tailscale serve identity header.
pub struct Authenticated;

impl FromRequestParts<AppState> for Authenticated {
    type Rejection = HubError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if resolve_machine(parts, state).is_ok() || trusted_tailscale_identity(parts, state) {
            Ok(Authenticated)
        } else {
            Err(HubError::Unauthorized)
        }
    }
}
