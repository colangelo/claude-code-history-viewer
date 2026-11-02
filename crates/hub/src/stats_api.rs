//! Statistics endpoints: `GET /v1/stats/global`, `/v1/stats/projects/{key}`,
//! `/v1/stats/sessions/{id}`, and the stats-read routes `/v1/stats/daily` and
//! `/v1/stats/tool-calls` (the latter reads Postgres: the mirror has no host).
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

use crate::auth::{Authenticated, Visibility};
use crate::error::HubError;
use crate::mirror::{Mirror, MirrorState};
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

#[doc(hidden)]
pub type StatsHeaders = [(HeaderName, String); 2];

/// One private connection to the statistics mirror per request, with the
/// readiness and the client-flag watermark read **through that same
/// connection** (security T2). The check in [`restrict`] and the aggregation in
/// [`MirrorSnapshot::run`] therefore see one database: a `hub mirror rebuild`
/// adopted between them swaps the mirror's own handle, not this one.
///
/// Every stats handler goes through here, which is what makes the module-doc
/// properties structural rather than remembered: the warming `503`, the
/// staleness headers, and the `spawn_blocking` hop. The connection is cloned
/// per request, which also keeps each request's `stats_scope` temp table
/// private to it.
#[doc(hidden)]
pub struct MirrorSnapshot {
    conn: Connection,
    refreshed_at: DateTime<Utc>,
    flags_as_of: Option<String>,
}

impl MirrorSnapshot {
    #[doc(hidden)]
    pub fn take(state: &AppState) -> Result<Self, HubError> {
        let mirror = state.mirror.as_ref().ok_or(warming())?;
        let conn = mirror
            .connection()
            .map_err(|e| HubError::Internal(format!("cloning a stats mirror connection: {e}")))?;
        // Read the state *before* the rollup so the headers describe the data
        // the response was computed from, not a refresh that landed while it ran.
        let MirrorState::Ready { refreshed_at, .. } = Mirror::state_on(&conn) else {
            return Err(warming());
        };
        let flags_as_of = Mirror::client_flags_as_of_on(&conn);
        Ok(Self {
            conn,
            refreshed_at,
            flags_as_of,
        })
    }

    /// The client-flag watermark of this snapshot's connection.
    #[doc(hidden)]
    pub fn flags_as_of(&self) -> Option<String> {
        self.flags_as_of.clone()
    }

    /// Run `f` on this snapshot's connection, on a blocking worker, and pair
    /// its result with the staleness headers. One blocking hop per request: `f`
    /// does reference resolution, identity expansion and the rollups inside it.
    #[doc(hidden)]
    pub async fn run<T, F>(self, f: F) -> Result<(StatsHeaders, T), HubError>
    where
        F: FnOnce(&Connection) -> Result<T, HubError> + Send + 'static,
        T: Send + 'static,
    {
        let Self {
            conn, refreshed_at, ..
        } = self;
        let out = tokio::task::spawn_blocking(move || f(&conn))
            .await
            .map_err(|e| HubError::Internal(format!("stats rollup task failed: {e}")))??;
        Ok((staleness_headers(refreshed_at), out))
    }
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
        Ok(Window {
            from,
            to,
            tz,
            sessions_visible: None,
        })
    }
}

/// The window limited to what `vis` lets the caller see (R2-E), checked
/// against `flags_as_of`, the watermark of the snapshot the request will
/// aggregate on (security T2). A caller that sees everything gets the window
/// unchanged.
#[doc(hidden)]
pub async fn restrict(
    state: &AppState,
    vis: Visibility,
    mut w: Window,
    flags_as_of: Option<String>,
) -> Result<Window, HubError> {
    if vis.sees_all() {
        return Ok(w);
    }
    // Refuse rather than serve a mirror that cannot prove the current flags.
    let behind: bool = sqlx::query_scalar(
        "SELECT $1::text IS NULL OR flags_changed_at > $1::timestamptz FROM client_scope_state",
    )
    .bind(flags_as_of)
    .fetch_one(&state.pool)
    .await?;
    if behind {
        return Err(HubError::MirrorBehind(
            "statistics mirror is older than the last client-scope change; retry after the next refresh".into(),
        ));
    }
    w.sessions_visible = Some(vis.session_ok("s"));
    Ok(w)
}

/// What a stats-read route's caller may see (D3 (a)), or 503 when a lab or
/// stats-only caller asks a hub that has no client scope to filter with
/// (security T3): fail closed rather than serve every session.
fn stats_read_visibility(state: &AppState, auth: &Authenticated) -> Result<Visibility, HubError> {
    Visibility::stats_read(state, auth.1).ok_or_else(|| {
        HubError::Unavailable(
            "client scope is not configured on this hub; the stats-read routes need it to serve a lab reader".into(),
        )
    })
}

/// Archive-wide statistics.
pub async fn global(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<StatsParams>,
) -> Result<(StatsHeaders, Json<GlobalStatsSummary>), HubError> {
    let w = params.window()?;
    let snap = MirrorSnapshot::take(&state)?;
    let w = restrict(&state, auth.visibility(&state), w, snap.flags_as_of()).await?;
    let (headers, summary) = snap.run(move |c| Ok(stats::global(c, &w)?)).await?;
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
    auth: Authenticated,
    State(state): State<AppState>,
    Path(identity_key): Path<String>,
    Query(params): Query<StatsParams>,
) -> Result<(StatsHeaders, Json<ProjectStatsSummary>), HubError> {
    let w = params.window()?;
    let snap = MirrorSnapshot::take(&state)?;
    let w = restrict(&state, auth.visibility(&state), w, snap.flags_as_of()).await?;
    let include_worktrees = params.include_worktrees.unwrap_or(true);
    let (headers, summary) = snap
        .run(move |c| {
            let mut paths = stats::resolve_identity_paths(c, &identity_key, include_worktrees)?;
            if let Some(visible) = &w.sessions_visible {
                // A lab reader's identity expands only to paths it may see; none
                // left is the same 404 as an unknown identity (R2-G).
                let seen = stats::visible_paths(c, visible)?;
                paths.retain(|p| seen.contains(p));
            }
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
    auth: Authenticated,
    State(state): State<AppState>,
    Path(session_ref): Path<String>,
    Query(params): Query<StatsParams>,
) -> Result<(StatsHeaders, Json<SessionTokenStats>), HubError> {
    let w = params.window()?;
    let snap = MirrorSnapshot::take(&state)?;
    let w = restrict(&state, auth.visibility(&state), w, snap.flags_as_of()).await?;
    let (headers, stats) = snap
        .run(move |c| {
            let absent = || HubError::NotFound(format!("no session {session_ref}"));
            let found = stats::resolve_session_ref(c, &session_ref)?;
            // Visibility before resolution (R2-G): hidden candidates do not exist.
            let found = match (&w.sessions_visible, found) {
                (Some(visible), SessionRef::Found(pk)) => {
                    if stats::visible_ids(c, visible, &[pk])?.is_empty() {
                        SessionRef::Absent
                    } else {
                        SessionRef::Found(pk)
                    }
                }
                (Some(visible), SessionRef::Ambiguous(ids)) => {
                    match stats::visible_ids(c, visible, &ids)?.as_slice() {
                        [] => SessionRef::Absent,
                        [pk] => SessionRef::Found(*pk),
                        many => SessionRef::Ambiguous(many.to_vec()),
                    }
                }
                (_, other) => other,
            };
            let pk = match found {
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

/// The longest window the stats-read routes answer, in days (stats-read-api
/// D3 condition 2: the counts cannot become an oracle over all of history).
pub const MAX_WINDOW_DAYS: i64 = 31;

/// `/v1/stats/daily` parameters: these three and no others (D3 condition 2).
/// An unknown parameter is a 400, so a filter on a project, a session or
/// content cannot be added by a caller and silently ignored.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyParams {
    /// Inclusive first local day, `YYYY-MM-DD`; default `to` minus 30 days.
    pub from: Option<String>,
    /// Inclusive last local day; default today in `tz`.
    pub to: Option<String>,
    /// IANA zone of the days; default `Europe/Rome`.
    pub tz: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct DailyAnswer {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub tz: String,
    pub rows: Vec<stats::DailyRow>,
}

impl DailyParams {
    fn window(&self) -> Result<Window, HubError> {
        let tz = parse_tz(self.tz.as_deref().unwrap_or("Europe/Rome"))?;
        let to = parse_date("to", self.to.as_ref())?
            .unwrap_or_else(|| Utc::now().with_timezone(&tz).date_naive());
        let from = parse_date("from", self.from.as_ref())?
            .unwrap_or(to - chrono::Duration::days(MAX_WINDOW_DAYS - 1));
        if from > to {
            return Err(HubError::BadRequest(format!(
                "from ({from}) is after to ({to})"
            )));
        }
        if (to - from).num_days() + 1 > MAX_WINDOW_DAYS {
            return Err(HubError::BadRequest(format!(
                "window {from}..{to} is longer than {MAX_WINDOW_DAYS} days"
            )));
        }
        Ok(Window {
            from: Some(from),
            to: Some(to),
            tz,
            sessions_visible: None,
        })
    }
}

/// Per local day and project: monitoring's dashboard rows (stats-read-api D2).
/// Lab sessions only for a lab reader, in observe as under enforce (D3 (a)).
pub async fn daily(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<DailyParams>,
) -> Result<(StatsHeaders, Json<DailyAnswer>), HubError> {
    let w = params.window()?;
    let vis = stats_read_visibility(&state, &auth)?;
    let snap = MirrorSnapshot::take(&state)?;
    let w = restrict(&state, vis, w, snap.flags_as_of()).await?;
    let (from, to, tz) = (
        w.from.expect("set by DailyParams::window"),
        w.to.expect("set by DailyParams::window"),
        w.tz.name().to_string(),
    );
    let (headers, rows) = snap
        .run(move |c| Ok(stats::daily_by_project(c, &w)?))
        .await?;
    Ok((headers, Json(DailyAnswer { from, to, tz, rows })))
}

/// `/v1/stats/tool-calls` parameters, these two and no others (D3 condition 2).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallsParams {
    /// A tool name, or the last `__` segment of one (`web_search_exa`).
    pub prefix: Option<String>,
    /// RFC 3339; at most [`MAX_WINDOW_DAYS`] back.
    pub since: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct ToolCount {
    pub tool: String,
    /// The MCP server of an `mcp__<server>__<tool>` name, else null.
    pub server: Option<String>,
    pub count: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct HostCount {
    pub host: String,
    pub count: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct ToolCallsAnswer {
    pub since: DateTime<Utc>,
    /// The newest message the caller may see at or after `since`: how fresh
    /// the counts are. Not the machines' `last_seen`, which client-only ingest
    /// moves (security T1).
    pub last_message_at: Option<DateTime<Utc>>,
    pub by_tool: Vec<ToolCount>,
    pub by_host: Vec<HostCount>,
}

fn mcp_server(tool: &str) -> Option<String> {
    let mut parts = tool.split("__");
    (parts.next() == Some("mcp"))
        .then(|| parts.next())
        .flatten()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Tool calls since T, per tool and per host (stats-read-api D4): quota-tap's
/// Exa counter. Lab sessions only for a lab reader, in observe as under
/// enforce (D3 (a)). Read from Postgres, where the archive is authoritative,
/// so no mirror freshness check applies.
pub async fn tool_calls(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(params): Query<ToolCallsParams>,
) -> Result<Json<ToolCallsAnswer>, HubError> {
    let prefix = params
        .prefix
        .filter(|p| {
            !p.is_empty()
                && p.len() <= 200
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        })
        .ok_or_else(|| HubError::BadRequest("prefix: a tool name (A-Z a-z 0-9 _ . -)".into()))?;
    let since = params
        .since
        .as_deref()
        .ok_or_else(|| HubError::BadRequest("since is required (RFC 3339)".into()))
        .and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .map(|t| t.with_timezone(&Utc))
                .map_err(|_| HubError::BadRequest(format!("since is not RFC 3339: {s}")))
        })?;
    if since < Utc::now() - chrono::Duration::days(MAX_WINDOW_DAYS) {
        return Err(HubError::BadRequest(format!(
            "since is more than {MAX_WINDOW_DAYS} days back"
        )));
    }
    let visible = stats_read_visibility(&state, &auth)?.session_ok("s");
    let sql = format!(
        r#"WITH hits AS (
               SELECT u.tool_name, m.machine_id
                 FROM messages m
                 JOIN message_tool_uses u ON u.message_ref = m.id
                 JOIN sessions s ON s.id = m.session_id
                WHERE m."timestamp" >= $1
                  AND (u.tool_name = $2 OR regexp_replace(u.tool_name, '^.*__', '') = $2)
                  AND {visible})
           SELECT 'tool', tool_name, count(*) FROM hits GROUP BY 2
           UNION ALL
           SELECT 'host', coalesce(mc.hostname, h.machine_id::text), count(*)
             FROM hits h LEFT JOIN machines mc ON mc.machine_id = h.machine_id
            GROUP BY 2"#
    );
    let rows: Vec<(String, String, i64)> = sqlx::query_as(&sql)
        .bind(since)
        .bind(&prefix)
        .fetch_all(&state.pool)
        .await?;
    // Security T1: the newest message the caller may see, not the machines'
    // last_seen, which moves on client-only ingest (a shared machine too).
    let last_message_at: Option<DateTime<Utc>> = sqlx::query_scalar(&format!(
        r#"SELECT m."timestamp" FROM messages m JOIN sessions s ON s.id = m.session_id
            WHERE m."timestamp" >= $1 AND {visible}
            ORDER BY m."timestamp" DESC LIMIT 1"#
    ))
    .bind(since)
    .fetch_optional(&state.pool)
    .await?
    .flatten();
    let (mut by_tool, mut by_host) = (Vec::new(), Vec::new());
    for (kind, key, count) in rows {
        if kind == "tool" {
            by_tool.push(ToolCount {
                server: mcp_server(&key),
                tool: key,
                count,
            });
        } else {
            by_host.push(HostCount { host: key, count });
        }
    }
    by_tool.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tool.cmp(&b.tool)));
    by_host.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.host.cmp(&b.host)));
    Ok(Json(ToolCallsAnswer {
        since,
        last_message_at,
        by_tool,
        by_host,
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn mcp_server_of_a_name() {
        assert_eq!(
            super::mcp_server("mcp__exa__web_search_exa").as_deref(),
            Some("exa")
        );
        assert_eq!(super::mcp_server("web_search_exa"), None);
        assert_eq!(super::mcp_server("mcp__"), None);
    }
}
