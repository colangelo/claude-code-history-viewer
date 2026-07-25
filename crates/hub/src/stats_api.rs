//! Statistics endpoints: `GET /v1/stats/global`, `/v1/stats/projects/{key}`,
//! `/v1/stats/sessions/{id}`.
//!
//! Thin HTTP layer over [`crate::stats`] — parameter parsing, identity
//! expansion and not-found handling. All the aggregation (and the dedup rule
//! that makes it correct) lives in that module.
//!
//! Responses are the stat types from `history-core` verbatim, which is what
//! lets the migrated frontend keep its TypeScript types and change only where
//! the data comes from (design D9).
//!
//! **Everything here is served from the `DuckDB` mirror, never from Postgres**
//! (design D4). Three consequences shape this file:
//!
//! * a mirror that has no data yet answers `503` + `Retry-After` rather than
//!   falling back to the 18 s Postgres rollups that no longer exist;
//! * every identifier a caller can pass — row id, session id, identity key —
//!   resolves against the mirror, so the endpoints stay answerable while
//!   Postgres is away, for all three scopes rather than just the easy one;
//! * the rollups are synchronous (`duckdb-rs` has no async API), so each request
//!   takes its own cloned connection onto `spawn_blocking`. Without that a
//!   ~0.4 s summary parks a tokio worker for its whole duration, and nothing in
//!   the correctness gates would ever notice.

use axum::extract::{Path, Query, State};
use axum::http::HeaderName;
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use duckdb::Connection;
use history_core::models::{GlobalStatsSummary, ProjectStatsSummary, SessionTokenStats};
use serde::Deserialize;

use crate::auth::Authenticated;
use crate::error::HubError;
use crate::mirror::MirrorState;
use crate::state::AppState;
use crate::stats::{self, SessionRef, Window};

/// How long to tell a caller to wait while the mirror builds.
///
/// The first cold build takes **~8 minutes** (497 s measured on m4m against
/// live pg1: 2,898,915 messages), so this is not "come back when it's done" but
/// "poll at a sane rate" — short enough that the webapp's Analytics tab recovers
/// on its own shortly after the mirror lands, rather than sitting on a stale
/// error until someone reloads.
const WARMING_RETRY_AFTER_SECS: u64 = 30;

/// When the mirror last completed a refresh, and how long ago that was.
///
/// Headers rather than body fields, deliberately: adding a field to the
/// `history-core` stat types breaks **struct literals** in `src-tauri` even when
/// it is serde-compatible, and `rust-tests.yml` builds that crate — a trap that
/// cost real time during `hub-analytics` (design D5). They must also be listed
/// in the router's CORS `expose_headers`, or the browser hides them from the
/// webapp that is the only consumer.
fn staleness_headers(refreshed_at: DateTime<Utc>) -> StatsHeaders {
    let age = (Utc::now() - refreshed_at).num_seconds().max(0);
    [
        (
            HeaderName::from_static("x-stats-mirror-refreshed-at"),
            refreshed_at.to_rfc3339(),
        ),
        (
            HeaderName::from_static("x-stats-mirror-age-seconds"),
            age.to_string(),
        ),
    ]
}

type StatsHeaders = [(HeaderName, String); 2];

/// Run `f` against a private connection to the statistics mirror, on a blocking
/// worker, and pair its result with the staleness headers.
///
/// Every stats handler goes through here, which is what makes the three
/// properties in the module docs structural rather than remembered: the warming
/// `503`, the staleness headers, and the `spawn_blocking` hop are impossible to
/// forget in a new endpoint because there is no other way to reach the mirror.
///
/// One blocking hop per request, not one per query: `f` receives the connection
/// and does all of its work — reference resolution, identity expansion, and the
/// rollups — inside it. The connection is cloned per request, which is also what
/// keeps each request's `stats_scope` temp table private to it.
async fn with_mirror<T, F>(state: &AppState, f: F) -> Result<(StatsHeaders, T), HubError>
where
    F: FnOnce(&Connection) -> Result<T, HubError> + Send + 'static,
    T: Send + 'static,
{
    let mirror = state.mirror.as_ref().ok_or(warming())?;
    // Read the state *before* the rollup so the headers describe the data the
    // response was computed from, not a refresh that landed while it ran.
    let MirrorState::Ready { refreshed_at, .. } = mirror.state() else {
        return Err(warming());
    };
    let conn = mirror
        .connection()
        .map_err(|e| HubError::Internal(format!("cloning a stats mirror connection: {e}")))?;
    let out = tokio::task::spawn_blocking(move || f(&conn))
        .await
        .map_err(|e| HubError::Internal(format!("stats rollup task failed: {e}")))??;
    Ok((staleness_headers(refreshed_at), out))
}

fn warming() -> HubError {
    HubError::StatsWarming {
        retry_after_secs: WARMING_RETRY_AFTER_SECS,
    }
}

#[derive(Debug, Deserialize)]
pub struct StatsParams {
    /// Inclusive lower bound, `YYYY-MM-DD`, in `tz`.
    pub from: Option<String>,
    /// Inclusive upper bound, `YYYY-MM-DD`, in `tz`.
    pub to: Option<String>,
    /// IANA timezone the day/hour buckets are expressed in. Defaults to UTC.
    pub tz: Option<String>,
    /// In identity scope: `false` excludes worktree-only member paths.
    pub include_worktrees: Option<bool>,
}

/// Resolve an IANA name against the compiled-in timezone database.
///
/// Stricter than the character-class check this replaces: `Not/A_Zone` passed
/// that and then produced statistics bucketed by whatever the database made of
/// it. Here an unknown zone is a `400` naming it, and everything downstream
/// carries a parsed `Tz` that cannot be invalid (see [`Window`]).
fn parse_tz(tz: &str) -> Result<chrono_tz::Tz, HubError> {
    crate::tz_spans::parse(tz)
        .ok_or_else(|| HubError::BadRequest(format!("invalid timezone: {tz}")))
}

fn parse_date(label: &str, raw: Option<&String>) -> Result<Option<NaiveDate>, HubError> {
    raw.map(|s| {
        s.parse::<NaiveDate>().map_err(|_| {
            HubError::BadRequest(format!("invalid {label} date (want YYYY-MM-DD): {s}"))
        })
    })
    .transpose()
}

impl StatsParams {
    fn window(&self) -> Result<Window, HubError> {
        let tz = parse_tz(self.tz.as_deref().unwrap_or("UTC"))?;
        let from = parse_date("from", self.from.as_ref())?;
        let to = parse_date("to", self.to.as_ref())?;
        if let (Some(f), Some(t)) = (from, to) {
            if f > t {
                return Err(HubError::BadRequest(format!(
                    "from ({f}) is after to ({t})"
                )));
            }
        }
        Ok(Window { from, to, tz })
    }
}

/// Archive-wide statistics.
pub async fn global(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<StatsParams>,
) -> Result<(StatsHeaders, Json<GlobalStatsSummary>), HubError> {
    let w = params.window()?;
    let (headers, summary) = with_mirror(&state, move |c| Ok(stats::global(c, &w)?)).await?;
    Ok((headers, Json(summary)))
}

/// Statistics for one project identity, folded across every path and machine
/// that belongs to it (design D6).
///
/// The identity expansion runs against the mirror rather than reusing the
/// browse-side Postgres one, so this endpoint needs no database at serve time.
/// Both halves are mirrored for it: the fingerprinted project rows and the
/// manual alias table.
pub async fn project(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(identity_key): Path<String>,
    Query(params): Query<StatsParams>,
) -> Result<(StatsHeaders, Json<ProjectStatsSummary>), HubError> {
    let w = params.window()?;
    let include_worktrees = params.include_worktrees.unwrap_or(true);
    let (headers, summary) = with_mirror(&state, move |c| {
        let paths = stats::resolve_identity_paths(c, &identity_key, include_worktrees)?;
        if paths.is_empty() {
            // An unknown identity expands to nothing. Returning zeroed
            // statistics would be indistinguishable from a real-but-idle
            // project, so this is a 404 rather than an empty success.
            return Err(HubError::NotFound(format!(
                "no project identity {identity_key}"
            )));
        }
        let name = stats::identity_display_name(c, &identity_key, &paths)?;
        Ok(stats::project(c, name, paths, &w)?)
    })
    .await?;
    Ok((headers, Json(summary)))
}

/// Statistics for one session, by hub row id or by provider session id
/// (Gitea #26).
pub async fn session(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(session_ref): Path<String>,
    Query(params): Query<StatsParams>,
) -> Result<(StatsHeaders, Json<SessionTokenStats>), HubError> {
    let w = params.window()?;
    let (headers, stats) = with_mirror(&state, move |c| {
        let absent = || HubError::NotFound(format!("no session {session_ref}"));
        let pk = match stats::resolve_session_ref(c, &session_ref)? {
            SessionRef::Found(pk) => pk,
            SessionRef::Absent => return Err(absent()),
            SessionRef::Ambiguous(ids) => {
                return Err(HubError::BadRequest(format!(
                    "session id {session_ref} is ambiguous across machines; use a numeric \
                     session id from /v1/sessions (candidates: {})",
                    ids.iter()
                        .map(i64::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )))
            }
        };
        stats::session(c, pk, &w)?.ok_or_else(absent)
    })
    .await?;
    Ok((headers, Json(stats)))
}
