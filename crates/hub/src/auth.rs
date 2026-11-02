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
//!
//! Reader tokens (`[[readers]]` in `hub.toml`) are the mirror image: accepted by
//! [`Authenticated`], refused by [`IngestMachine`] and [`AuthedMachine`], and
//! refused by [`refuse_read_only_writes`] on every method but GET, HEAD and
//! OPTIONS. That router layer is the default deny for writes: the identity
//! alias writes take `Authenticated` (a trusted Tailscale identity makes them),
//! and so might a write added later, so the method decides, not the extractor.

use std::convert::Infallible;
use std::future::{ready, Future};

use axum::extract::{FromRequestParts, OptionalFromRequestParts, Request, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::error::HubError;
use crate::state::AppState;
pub use crate::visibility::{ReadScope, Visibility};

/// What the calling extractor is about to allow.
///
/// | token kind  | Ingest | Write | Read |
/// |-------------|--------|-------|------|
/// | full        | yes    | yes   | yes  |
/// | ingest_only | yes    | no    | no   |
/// | read_only   | no     | no    | yes  |
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// `/v1/ingest`: writes under the token's own machine id.
    Ingest,
    /// The other machine-token writes (journal entries, distiller ticks).
    Write,
    /// The read endpoints.
    Read,
}

/// The bearer token in the request headers, if any (trimmed, non-empty).
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Resolve the bearer token from request headers to its machine id, or 401
/// when the token is unknown or its kind does not allow `scope`.
fn resolve_machine(parts: &Parts, state: &AppState, scope: Scope) -> Result<Uuid, HubError> {
    resolve_machine_headers(&parts.headers, state, scope)
}

fn resolve_machine_headers(
    headers: &HeaderMap,
    state: &AppState,
    scope: Scope,
) -> Result<Uuid, HubError> {
    let token = bearer(headers).ok_or(HubError::Unauthorized)?;
    let refused = match scope {
        Scope::Ingest => state.read_only.contains(token),
        Scope::Write => state.ingest_only.contains(token) || state.read_only.contains(token),
        Scope::Read => state.ingest_only.contains(token),
    };
    if refused {
        return Err(HubError::Unauthorized);
    }
    state
        .tokens
        .get(token)
        .copied()
        .ok_or(HubError::Unauthorized)
}

/// Router layer: a read-only token on any method but GET, HEAD or OPTIONS is
/// 401 before routing, so every write route — registered today or later,
/// whatever extractor it takes — refuses it. Other requests pass untouched.
pub async fn refuse_read_only_writes(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    // `/v1/login` exchanges a read token for a read-only cookie: not a write.
    let reads = matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) || request.uri().path() == "/v1/login";
    let read_only = |t: &str| state.read_only.contains(t) || client_reader(&state, t).is_some();
    if !reads && bearer(request.headers()).is_some_and(read_only) {
        return HubError::Unauthorized.into_response();
    }
    next.run(request).await
}

/// The paths a `stats_only` reader may request (stats-read-api D3 c3).
pub const STATS_ONLY_PATHS: [&str; 2] = ["/v1/stats/daily", "/v1/stats/tool-calls"];

/// Router layer: a `stats_only` reader token gets 403 on every request but a
/// GET of [`STATS_ONLY_PATHS`] — `/v1/login` included, so it cannot become a
/// cookie that reads elsewhere. Outermost of the auth layers, so a route added
/// later is refused without code of its own.
pub async fn refuse_stats_only_elsewhere(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let allowed = matches!(*request.method(), Method::GET | Method::HEAD)
        && STATS_ONLY_PATHS.contains(&request.uri().path());
    if !allowed
        && bearer(request.headers()).is_some_and(|t| state.stats_only.contains(t))
    {
        return HubError::Forbidden("this token reads the stats routes only".into())
            .into_response();
    }
    next.run(request).await
}

/// The machine id behind a token that may ingest (full or ingest-only; a
/// read-only token is refused). Ingest only.
pub struct IngestMachine(pub Uuid);

impl FromRequestParts<AppState> for IngestMachine {
    type Rejection = HubError;

    // Synchronous, like `AuthedMachine` below — see the note there.
    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        ready(resolve_machine(parts, state, Scope::Ingest).map(IngestMachine))
    }
}

/// The machine id behind a full-scope token (ingest-only and read-only tokens
/// are refused).
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
        ready(resolve_machine(parts, state, Scope::Write).map(AuthedMachine))
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
/// full-scope or read-only bearer token, or (opt-in) a trusted Tailscale serve
/// identity header. The identity alias writes take it too; a read-only token
/// never reaches them (see [`refuse_read_only_writes`]).
///
/// Carries the caller's principal (`machine:<uuid>` / `tailscale:<login>`)
/// for audit trails on the few read-principal writes (identity aliases).
pub struct Authenticated(pub String, pub ReadScope);

impl Authenticated {
    /// What this caller may see, under the hub's client-scope state now.
    pub fn visibility(&self, state: &AppState) -> Visibility {
        Visibility::of(state, self.1)
    }
}

/// The machine id of a client-read principal, by the sha256 of its token
/// (the hub keeps only hashes of client-read tokens, design R2-H).
fn client_reader(state: &AppState, token: &str) -> Option<Uuid> {
    if state.client_readers.is_empty() {
        return None;
    }
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let hash =
        Sha256::digest(token.as_bytes())
            .iter()
            .fold(String::with_capacity(64), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            });
    state.client_readers.get(&hash).copied()
}

/// Resolve the caller's read principal: a valid read-capable bearer token first,
/// then a trusted Tailscale identity header, else 401.
/// A read principal from the bearer token alone (no cookie, no Tailscale
/// identity): what `/v1/login` accepts.
pub fn bearer_read_principal(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<Authenticated, HubError> {
    if let Ok(machine) = resolve_machine_headers(headers, state, Scope::Read) {
        return Ok(Authenticated(format!("machine:{machine}"), ReadScope::Lab));
    }
    if let Some(machine) = bearer(headers).and_then(|t| client_reader(state, t)) {
        return Ok(Authenticated(
            format!("client-reader:{machine}"),
            ReadScope::Client,
        ));
    }
    Err(HubError::Unauthorized)
}

fn resolve_read_principal(parts: &Parts, state: &AppState) -> Result<Authenticated, HubError> {
    if let Ok(found) = bearer_read_principal(&parts.headers, state) {
        return Ok(found);
    }
    // A browser session from `/v1/login` (R2-H).
    if let Some(found) =
        crate::login::cookie_value(&parts.headers).and_then(|v| state.logins.lookup(v))
    {
        return Ok(found);
    }
    // The Tailscale identity is lab read, always: the hub cannot tell ac's
    // browser from any other process of his login (design D5).
    if let Some(login) = trusted_tailscale_identity(parts, state) {
        return Ok(Authenticated(format!("tailscale:{login}"), ReadScope::Lab));
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

/// `Option<Authenticated>`: for the health routes, which answer everyone but
/// show detail only to a read principal. A missing, unknown or non-read token is
/// `None`, never a 401: the verdict stays reachable for the monitor.
impl OptionalFromRequestParts<AppState> for Authenticated {
    type Rejection = Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Option<Self>, Self::Rejection>> + Send {
        ready(Ok(resolve_read_principal(parts, state).ok()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// The extractors' own table, without the router layer in front of them:
    /// each guarantee holds even if the layer were not mounted.
    #[tokio::test]
    async fn every_token_kind_gets_exactly_its_scopes() {
        let (full, ingest, read) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let tokens = HashMap::from([
            ("t-full".to_string(), full),
            ("t-ingest".to_string(), ingest),
            ("t-read".to_string(), read),
        ]);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@localhost/unused")
            .unwrap();
        let state = AppState::new(pool, tokens, Vec::new())
            .with_ingest_only_tokens(HashSet::from(["t-ingest".to_string()]))
            .with_read_only_tokens(HashSet::from(["t-read".to_string()]));
        let parts = |token: &str| {
            let (parts, ()) = axum::http::Request::builder()
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .body(())
                .unwrap()
                .into_parts();
            parts
        };
        let ok = |token: &str, scope| resolve_machine(&parts(token), &state, scope).ok();
        // (token, ingest, write, read)
        let table = [
            ("t-full", Some(full), Some(full), Some(full)),
            ("t-ingest", Some(ingest), None, None),
            ("t-read", None, None, Some(read)),
            ("t-unknown", None, None, None),
        ];
        for (token, i, w, r) in table {
            assert_eq!(ok(token, Scope::Ingest), i, "{token} ingest");
            assert_eq!(ok(token, Scope::Write), w, "{token} write");
            assert_eq!(ok(token, Scope::Read), r, "{token} read");
        }
    }
}
