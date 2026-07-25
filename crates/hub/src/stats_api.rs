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

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::NaiveDate;
use history_core::models::{GlobalStatsSummary, ProjectStatsSummary, SessionTokenStats};
use serde::Deserialize;

use crate::auth::Authenticated;
use crate::error::HubError;
use crate::identity_filter::{self, IDENTITY_PREFIX};
use crate::state::AppState;
use crate::stats::{self, Window};

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
) -> Result<Json<GlobalStatsSummary>, HubError> {
    let w = params.window()?;
    Ok(Json(stats::global(&state.pool, &w).await?))
}

/// Statistics for one project identity, folded across every path and machine
/// that belongs to it (design D6).
pub async fn project(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(identity_key): Path<String>,
    Query(params): Query<StatsParams>,
) -> Result<Json<ProjectStatsSummary>, HubError> {
    let w = params.window()?;

    // Reuse the browse-side expansion so "one project" means the same thing
    // here as everywhere else: fingerprinted rows sharing the key, plus
    // manually aliased (typically moved-away) paths.
    let scope = identity_filter::resolve_project_scope(
        &state.pool,
        Some(&format!("{IDENTITY_PREFIX}{identity_key}")),
        params.include_worktrees.unwrap_or(true),
    )
    .await?;

    let paths = scope.paths.unwrap_or_default();
    if paths.is_empty() {
        // An unknown identity expands to nothing. Returning zeroed statistics
        // would be indistinguishable from a real-but-idle project, so this is a
        // 404 rather than an empty success.
        return Err(HubError::NotFound(format!(
            "no project identity {identity_key}"
        )));
    }

    let name = display_name(&state.pool, &identity_key, &paths).await?;
    Ok(Json(stats::project(&state.pool, name, paths, &w).await?))
}

/// A human label for the identity: its most common project name, else the
/// shortest member path (the main checkout, rather than a worktree).
async fn display_name(
    pool: &sqlx::PgPool,
    identity_key: &str,
    paths: &[String],
) -> Result<String, HubError> {
    let name: Option<String> = sqlx::query_scalar(
        r"
        SELECT name FROM projects
         WHERE identity_key = $1 AND name IS NOT NULL
         GROUP BY name ORDER BY count(*) DESC LIMIT 1
        ",
    )
    .bind(identity_key)
    .fetch_optional(pool)
    .await?
    .flatten();

    Ok(name.unwrap_or_else(|| {
        paths
            .iter()
            .min_by_key(|p| p.len())
            .cloned()
            .unwrap_or_else(|| identity_key.to_string())
    }))
}

/// Statistics for one session, by surrogate id.
pub async fn session(
    _auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(params): Query<StatsParams>,
) -> Result<Json<SessionTokenStats>, HubError> {
    let w = params.window()?;
    stats::session(&state.pool, id, &w)
        .await?
        .map(Json)
        .ok_or_else(|| HubError::NotFound(format!("no session {id}")))
}
