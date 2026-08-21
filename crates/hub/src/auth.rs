//! Bearer-token authentication.
//!
//! Two extractors gate handlers on a valid `Authorization: Bearer <token>`
//! header (a missing/unknown token yields 401):
//! - [`AuthedMachine`] resolves the token to a machine id — used by ingest,
//!   which writes under that machine's identity.
//! - [`Authenticated`] only proves the caller holds a valid token — used by the
//!   read endpoints, which span all machines in the archive.

use std::future::{ready, Future};

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

    // Not an `async fn`: both extractors resolve synchronously — a header read
    // plus a map lookup, no I/O — so they hand back an already-complete future
    // instead of a future that never awaits (`clippy::unused_async_trait_impl`,
    // which is `-D warnings` in CI). The decision itself stays in the plain
    // functions above, which is where it is readable and testable.
    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        ready(resolve_machine(parts, state).map(AuthedMachine))
    }
}

/// Header injected by Tailscale serve for tailnet clients (absent on Funnel
/// traffic). Verified by serve itself; the hub trusts it only for logins in
/// the configured allow-list.
const TAILSCALE_USER_LOGIN: &str = "tailscale-user-login";

/// The trusted `Tailscale-User-Login` value when the request carries one that
/// matches the configured allow-list. Grants READ scope only — ingest never
/// calls this.
fn trusted_tailscale_identity(parts: &Parts, state: &AppState) -> Option<String> {
    if state.trusted_identities.is_empty() {
        return None;
    }
    parts
        .headers
        .get(TAILSCALE_USER_LOGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|login| {
            state
                .trusted_identities
                .iter()
                .any(|t| t.eq_ignore_ascii_case(login))
        })
        .map(str::to_string)
}

/// Proof that the caller may read. Read endpoints query across all machines,
/// so they need authentication but not a bound machine identity: a valid
/// bearer token, or (opt-in) a trusted Tailscale serve identity header.
///
/// Carries the caller's principal (`machine:<uuid>` / `tailscale:<login>`)
/// for audit trails on the few read-principal writes (identity aliases).
pub struct Authenticated(pub String);

/// Resolve the caller's read principal: a valid bearer token first, then a
/// trusted Tailscale identity header, else 401.
fn resolve_read_principal(parts: &Parts, state: &AppState) -> Result<Authenticated, HubError> {
    if let Ok(machine) = resolve_machine(parts, state) {
        return Ok(Authenticated(format!("machine:{machine}")));
    }
    if let Some(login) = trusted_tailscale_identity(parts, state) {
        return Ok(Authenticated(format!("tailscale:{login}")));
    }
    Err(HubError::Unauthorized)
}

impl FromRequestParts<AppState> for Authenticated {
    type Rejection = HubError;

    // Synchronous, like `AuthedMachine` above — see the note there.
    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        ready(resolve_read_principal(parts, state))
    }
}
