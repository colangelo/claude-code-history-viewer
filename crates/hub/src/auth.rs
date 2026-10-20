//! Bearer-token authentication.
//!
//! Three extractors gate handlers on a valid `Authorization: Bearer <token>`
//! header (a missing/unknown token yields 401):
//! - [`IngestMachine`] resolves the token to a machine id — used by ingest
//!   only, which writes under that machine's identity. It is the one extractor
//!   that accepts an ingest-only token.
//! - [`AuthedMachine`] resolves a full-scope token to a machine id — used by the
//!   other machine-token writes (journal entries, distiller ticks).
//! - [`Authenticated`] proves the caller holds a full-scope token or a trusted
//!   tailnet identity — used by the read endpoints, which span all machines.
//!
//! Ingest-only tokens (`ingest_only = true` in `hub.toml`) are refused by
//! everything except [`IngestMachine`], so a fenced guest can archive its own
//! sessions without reading the archive. The default is deny: a new route that
//! takes `AuthedMachine` or `Authenticated` refuses them without extra code.

use std::future::{ready, Future};

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::error::HubError;
use crate::state::AppState;

/// Resolve the bearer token from request headers to its machine id, or 401.
/// An ingest-only token resolves only when `allow_ingest_only` is set.
fn resolve_machine(
    parts: &Parts,
    state: &AppState,
    allow_ingest_only: bool,
) -> Result<Uuid, HubError> {
    let token = parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or(HubError::Unauthorized)?;
    if !allow_ingest_only && state.ingest_only.contains(token) {
        return Err(HubError::Unauthorized);
    }
    state
        .tokens
        .get(token)
        .copied()
        .ok_or(HubError::Unauthorized)
}

/// The machine id behind any valid token, ingest-only included. Ingest only.
pub struct IngestMachine(pub Uuid);

impl FromRequestParts<AppState> for IngestMachine {
    type Rejection = HubError;

    // Synchronous, like `AuthedMachine` below — see the note there.
    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        ready(resolve_machine(parts, state, true).map(IngestMachine))
    }
}

/// The machine id behind a full-scope token (ingest-only tokens are refused).
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
        ready(resolve_machine(parts, state, false).map(AuthedMachine))
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

/// Resolve the caller's read principal: a valid full-scope bearer token first,
/// then a trusted Tailscale identity header, else 401.
fn resolve_read_principal(parts: &Parts, state: &AppState) -> Result<Authenticated, HubError> {
    if let Ok(machine) = resolve_machine(parts, state, false) {
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
