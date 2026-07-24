//! `GET /v1/healthz` — unauthenticated liveness + database connectivity check.
//! `GET /v1/healthz/ingest` — unauthenticated per-machine ingest-daemon
//! liveness, so Gatus (HTTP status/body only) can alert on a dead daemon even
//! while `/v1/healthz` stays green.
//! `GET /v1/healthz/journal` — unauthenticated journal-distillation staleness,
//! so the same monitor can alert when closed days sit undrained (the pipeline
//! stalled) even while both checks above stay green.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;

use crate::error::HubError;
use crate::state::AppState;

pub async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        Ok(_) => (StatusCode::OK, Json(json!({ "status": "ok", "db": "up" }))),
        Err(e) => {
            tracing::error!(error = %e, "healthz db check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "degraded", "db": "down" })),
            )
        }
    }
}

/// Daemons scan hourly; the default threshold is 2x that.
const DEFAULT_STALE_AFTER_SECS: i64 = 7200;

#[derive(Debug, Deserialize)]
pub struct IngestHealthParams {
    /// Raw string, not `i64`: non-numeric input must 400 rather than fail
    /// Axum's query-extraction rejection, so it goes through `HubError` like
    /// every other validation failure in this crate.
    pub stale_after_secs: Option<String>,
    /// Comma-separated hostnames to drop from the alert verdict (e.g. a
    /// decommissioning machine whose dead daemon is expected). Excluded
    /// machines are still reported for observability but never flip the
    /// endpoint to 503. Matching is case-insensitive and tolerant of the mDNS
    /// `.local` suffix (so `ac-mbp` matches the stored `ac-mbp.local`). Keeping
    /// this a query param leaves the monitoring policy in Gatus's check config —
    /// no hub redeploy to change the set.
    pub exclude: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IngestMachineHealth {
    pub machine_id: Uuid,
    pub hostname: String,
    pub last_seen: DateTime<Utc>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub stale: bool,
    /// True when this machine's `hostname` is in the `exclude` set — its
    /// `stale` flag is still computed and reported, but it does not count
    /// toward the endpoint's overall stale/503 verdict.
    pub excluded: bool,
}

#[derive(Debug, Serialize)]
pub struct IngestHealthResponse {
    pub status: &'static str,
    pub stale_after_secs: i64,
    pub machines: Vec<IngestMachineHealth>,
}

fn parse_stale_after_secs(raw: Option<&str>) -> Result<i64, HubError> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_STALE_AFTER_SECS);
    };
    match raw.parse::<i64>() {
        Ok(secs) if secs > 0 => Ok(secs),
        _ => Err(HubError::BadRequest(format!(
            "stale_after_secs must be a positive integer, got {raw:?}"
        ))),
    }
}

/// Normalizes a hostname for exclude-set matching: trimmed, lowercased, with a
/// single trailing `.local` (mDNS) suffix stripped. The archive stores mDNS
/// names (`ac-mbp.local`), but operators — and the relay/docs — refer to the
/// machine as `ac-mbp`; normalizing both the stored hostname and each exclude
/// entry the same way lets `?exclude=ac-mbp` match `ac-mbp.local` without the
/// operator needing to know the suffix.
fn normalize_host(h: &str) -> String {
    let h = h.trim().to_ascii_lowercase();
    match h.strip_suffix(".local") {
        Some(stripped) => stripped.to_string(),
        None => h,
    }
}

/// Parses the `exclude` query param into a normalized hostname set (see
/// `normalize_host`). Empty entries are dropped so `?exclude=` or trailing
/// commas are harmless.
fn parse_exclude(raw: Option<&str>) -> std::collections::HashSet<String> {
    raw.map(|s| {
        s.split(',')
            .map(normalize_host)
            .filter(|h| !h.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

/// Staleness is judged on `machines.last_seen` alone (the daemon's
/// ingest-upsert heartbeat) — never on message recency, so an idle machine
/// with no new coding sessions doesn't page anyone.
pub async fn healthz_ingest(
    State(state): State<AppState>,
    Query(params): Query<IngestHealthParams>,
) -> Result<(StatusCode, Json<IngestHealthResponse>), HubError> {
    let stale_after_secs = parse_stale_after_secs(params.stale_after_secs.as_deref())?;
    let exclude = parse_exclude(params.exclude.as_deref());

    let rows = sqlx::query!(
        r#"
        SELECT mac.machine_id AS "machine_id!",
               mac.hostname   AS "hostname!",
               mac.last_seen  AS "last_seen!",
               lm.last_message_at
        FROM machines mac
        LEFT JOIN (
            SELECT machine_id, MAX(created_at) AS last_message_at
            FROM messages
            GROUP BY machine_id
        ) lm ON lm.machine_id = mac.machine_id
        ORDER BY mac.machine_id
        "#
    )
    .fetch_all(&state.pool)
    .await?;

    let now = Utc::now();
    let threshold = chrono::Duration::seconds(stale_after_secs);
    let mut any_stale = false;
    let machines = rows
        .into_iter()
        .map(|r| {
            let stale = now - r.last_seen > threshold;
            let excluded = exclude.contains(&normalize_host(&r.hostname));
            // Excluded machines report their real `stale` flag but never
            // contribute to the overall alert verdict.
            any_stale |= stale && !excluded;
            IngestMachineHealth {
                machine_id: r.machine_id,
                hostname: r.hostname,
                last_seen: r.last_seen,
                last_message_at: r.last_message_at,
                stale,
                excluded,
            }
        })
        .collect();

    let status = if any_stale {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(IngestHealthResponse {
            status: if any_stale { "stale" } else { "ok" },
            stale_after_secs,
            machines,
        }),
    ))
}

// ---------------------------------------------------------------------------
// GET /v1/healthz/journal
// ---------------------------------------------------------------------------

/// Grace after a group's latest data arrival before undrained work counts as a
/// stall. Default 2h = 2x the hourly distiller tick, matching the ingest
/// check's 2x-scan-interval heuristic: a day re-dirtied by a late machine wake
/// is legitimately pending for up to a tick, so grace keeps it green while the
/// next tick drains it.
const DEFAULT_JOURNAL_GRACE_SECS: i64 = 7200;

/// Only closed days within this many days of the current logical day are
/// evaluated — matching the distiller's forward `--horizon-days` (default 7).
/// Older pending groups are awaiting an explicit `--backfill`, are never
/// auto-distilled, and must not flip the check stale (the archive routinely
/// holds hundreds of them going back months).
const DEFAULT_JOURNAL_WITHIN_DAYS: i32 = 7;

#[derive(Debug, Deserialize)]
pub struct JournalHealthParams {
    /// Raw strings (not `i64`/`i32`): non-numeric input must 400 through
    /// `HubError` like every other validation failure, not Axum's opaque
    /// query-rejection. See [`parse_positive`].
    pub grace_secs: Option<String>,
    pub within_days: Option<String>,
}

/// One in-window pending `(entry_date, project_path)` group, with its latest
/// data arrival and whether that arrival is now older than the grace window.
#[derive(Debug, Serialize)]
pub struct JournalStaleGroup {
    pub entry_date: NaiveDate,
    pub project_path: String,
    /// `max(messages.created_at)` over the group's sessions — when its data
    /// last *arrived in the archive* (ingest time), not when it was authored.
    pub latest_arrival: DateTime<Utc>,
    pub stale: bool,
}

/// Raw row from the pending-group-with-arrival query; `stale` is derived in
/// Rust (like `healthz_ingest`) so the boundary logic is unit-testable.
#[derive(Debug, FromRow)]
struct JournalGroupRow {
    entry_date: NaiveDate,
    project_path: String,
    latest_arrival: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct JournalHealthResponse {
    pub status: &'static str,
    pub grace_secs: i64,
    pub within_days: i32,
    pub groups: Vec<JournalStaleGroup>,
}

/// Parse an optional positive-integer query param, or fall back to `default`.
/// Non-numeric / non-positive input becomes a `400` naming the parameter —
/// same contract as `parse_stale_after_secs`.
fn parse_positive(raw: Option<&str>, name: &str, default: i64) -> Result<i64, HubError> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    match raw.parse::<i64>() {
        Ok(v) if v > 0 => Ok(v),
        _ => Err(HubError::BadRequest(format!(
            "{name} must be a positive integer, got {raw:?}"
        ))),
    }
}

/// Journal-distillation staleness. A closed logical day within the forward
/// horizon (`within_days`) that still has pending groups — no journal row, or a
/// row whose snapshot predates a session's ingest (dirty) — whose latest data
/// arrived more than `grace_secs` ago is a stall, and flips the endpoint to
/// 503. The closed-day fold and pending semantics mirror
/// [`crate::journal::pending`] exactly (same [`crate::journal::DAY_START_HOUR`]);
/// the only addition is the per-group latest arrival and the horizon bound.
pub async fn healthz_journal(
    State(state): State<AppState>,
    Query(params): Query<JournalHealthParams>,
) -> Result<(StatusCode, Json<JournalHealthResponse>), HubError> {
    let grace_secs = parse_positive(
        params.grace_secs.as_deref(),
        "grace_secs",
        DEFAULT_JOURNAL_GRACE_SECS,
    )?;
    let within_days_i64 = parse_positive(
        params.within_days.as_deref(),
        "within_days",
        i64::from(DEFAULT_JOURNAL_WITHIN_DAYS),
    )?;
    let within_days = i32::try_from(within_days_i64).map_err(|_| {
        HubError::BadRequest(format!("within_days too large, got {within_days_i64}"))
    })?;

    // Runtime query (not `query!`): the offline gate has no `.sqlx` metadata for
    // new statements — same reason every query in `journal.rs` is runtime.
    //
    // `sess_win` filters to in-window closed days BEFORE the messages join so
    // `arrivals` scans only the last `within_days` of sessions, not the whole
    // archive. `grp` carries the per-session ingest xids for the dirty check;
    // `arrivals` carries the latest arrival. A group is pending when it has no
    // journal row, or a session's ingest xid is invisible in the row's snapshot
    // (committed after the entry was generated) — commit-order exact, identical
    // to `journal::pending`.
    let rows = sqlx::query_as::<_, JournalGroupRow>(
        r"
        WITH sess AS (
            SELECT
                s.id                                     AS session_id,
                ((s.first_message_time - make_interval(hours => $1::int))
                    AT TIME ZONE 'UTC')::date            AS entry_date,
                p.project_path                           AS project_path,
                s.ingest_xid                             AS ingest_xid
            FROM sessions s
            JOIN projects p ON s.project_id = p.id
            WHERE s.first_message_time IS NOT NULL
        ),
        sess_win AS (
            SELECT *
            FROM sess
            WHERE entry_date
                    < ((now() - make_interval(hours => $1::int)) AT TIME ZONE 'UTC')::date
              AND entry_date
                    >= ((now() - make_interval(hours => $1::int)) AT TIME ZONE 'UTC')::date
                        - $2::int
        ),
        grp AS (
            SELECT entry_date, project_path,
                   array_agg(ingest_xid) AS ingest_xids
            FROM sess_win
            GROUP BY entry_date, project_path
        ),
        arrivals AS (
            SELECT sw.entry_date, sw.project_path,
                   max(m.created_at) AS latest_arrival
            FROM sess_win sw
            JOIN messages m ON m.session_id = sw.session_id
            GROUP BY sw.entry_date, sw.project_path
        )
        SELECT g.entry_date, g.project_path, a.latest_arrival
        FROM grp g
        JOIN arrivals a
            ON a.entry_date = g.entry_date AND a.project_path = g.project_path
        LEFT JOIN journal_entries j
            ON j.entry_date = g.entry_date AND j.project_path = g.project_path
        WHERE j.id IS NULL OR EXISTS (
                SELECT 1 FROM unnest(g.ingest_xids) AS x
                WHERE NOT pg_visible_in_snapshot(x, j.generated_snapshot))
        ORDER BY g.entry_date DESC, g.project_path DESC
        ",
    )
    .bind(crate::journal::DAY_START_HOUR)
    .bind(within_days)
    .fetch_all(&state.pool)
    .await?;

    let now = Utc::now();
    let threshold = chrono::Duration::seconds(grace_secs);
    let mut any_stale = false;
    let groups = rows
        .into_iter()
        .map(|r| {
            let stale = now - r.latest_arrival > threshold;
            any_stale |= stale;
            JournalStaleGroup {
                entry_date: r.entry_date,
                project_path: r.project_path,
                latest_arrival: r.latest_arrival,
                stale,
            }
        })
        .collect();

    let status = if any_stale {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(JournalHealthResponse {
            status: if any_stale { "stale" } else { "ok" },
            grace_secs,
            within_days,
            groups,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_positive_defaults_when_absent() {
        assert_eq!(parse_positive(None, "grace_secs", 7200).unwrap(), 7200);
    }

    #[test]
    fn parse_positive_accepts_valid() {
        assert_eq!(parse_positive(Some("30"), "grace_secs", 7200).unwrap(), 30);
    }

    #[test]
    fn parse_positive_rejects_non_numeric() {
        let err = parse_positive(Some("abc"), "grace_secs", 7200).unwrap_err();
        assert!(matches!(err, HubError::BadRequest(m) if m.contains("grace_secs")));
    }

    #[test]
    fn parse_positive_rejects_zero_and_negative() {
        assert!(parse_positive(Some("0"), "grace_secs", 7200).is_err());
        assert!(parse_positive(Some("-1"), "within_days", 7).is_err());
    }
}
