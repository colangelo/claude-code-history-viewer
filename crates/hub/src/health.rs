//! `GET /v1/healthz` — unauthenticated liveness + database connectivity check.
//! `GET /v1/healthz/ingest` — unauthenticated per-machine ingest-daemon
//! liveness, so Gatus (HTTP status/body only) can alert on a dead daemon even
//! while `/v1/healthz` stays green.
//! `GET /v1/healthz/journal` — unauthenticated journal-distillation staleness,
//! so the same monitor can alert when closed days sit undrained (the pipeline
//! stalled) even while both checks above stay green — **and distiller-tick
//! liveness beside it**, because undrained work alone cannot say whether a
//! distiller is behind or absent.
//! `GET /v1/healthz/stats` — unauthenticated statistics-mirror readiness,
//! staleness **and watermark lag**, because a mirror can be recently refreshed
//! and still be missing rows.

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
use crate::mirror::MirrorState;
use crate::state::AppState;

/// The release this binary was built at — `package.json`'s version, inherited
/// through `[workspace.package]` by `just sync-version`. Exact semver; consumers
/// MUST compare it as such (`0.18.1` is a prefix of `0.18.10`).
pub const HUB_VERSION: &str = env!("CARGO_PKG_VERSION");

pub async fn healthz(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        // `version` is the workspace version `just sync-version` derives from
        // package.json, so a swap is proven by reading the running build rather
        // than by guessing it from which routes answer (#39). Present in both
        // arms: a degraded hub still has an identity.
        Ok(_) => (
            StatusCode::OK,
            Json(json!({ "status": "ok", "db": "up", "version": HUB_VERSION })),
        ),
        Err(e) => {
            tracing::error!(error = %e, "healthz db check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "degraded", "db": "down", "version": HUB_VERSION })),
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
    /// Age past which the last distiller tick flips this check to `no_tick`.
    /// **Absent by default, and then tick age is reported but never alerts** —
    /// the same rule as `healthz_stats`'s `max_lag_rows`, and for a sharper
    /// reason: the host running the distiller sleeps dozens of times a day, and
    /// launchd does not fire `StartInterval` while it is asleep, so any default
    /// threshold here would be an assumption about that host's wake schedule
    /// dressed up as a property of the archive. Keeping it a query param leaves
    /// the policy in the monitor's config, where it can be tuned without a hub
    /// redeploy (same reasoning as `healthz_ingest`'s `exclude`).
    pub max_tick_age_secs: Option<String>,
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
    /// `ok` · `stale` (in-window groups undrained past grace) · `no_tick` (no
    /// distiller tick within `max_tick_age_secs`, only reachable when that param
    /// is supplied).
    pub status: &'static str,
    pub grace_secs: i64,
    pub within_days: i32,
    /// When a distiller last told us it ran. `null` until one does — either
    /// because none has, or because the deployed distiller predates
    /// `POST /v1/journal/ticks`.
    pub last_tick_at: Option<DateTime<Utc>>,
    pub last_tick_age_secs: Option<i64>,
    /// `forward` (the scheduled job) or `backfill` (a hand-run historical pass).
    pub last_tick_mode: Option<String>,
    /// Groups that tick found pending when it started — the size of its work
    /// list, not of the work it finished.
    pub last_tick_groups_pending: Option<i32>,
    /// **This is why a last-tick timestamp alone is not enough.**
    /// `StartInterval 3600` does not mean 24 ticks a day, for two reasons that
    /// compound. The interval re-arms at the previous run's **exit**, so even an
    /// always-awake host is capped at `86400 / (3600 + duration)` — ≈21.7/day at
    /// the hub machine's median run. And on a host that sleeps, launchd
    /// coalesces every interval missed while asleep into a single catch-up. A
    /// count here well under 24 is the drain rate actually on offer, and it is
    /// what turns "the backlog should clear in N hours" into a question about
    /// ticks rather than clocks.
    ///
    /// It is **not** a wake count, and a reader who treats it as one is wrong in
    /// both directions. Measured on the hub machine over 13 days (2026-08-21):
    /// 15.38 ticks/day against 40–106 sleep cycles a day on that same host. Most
    /// of those are `DarkWake`s — which *do* run a `StartInterval` agent, an
    /// earlier version of this comment said otherwise and was wrong; what gates a
    /// catch-up is a delay of 28 s to 40 min after the box comes up, not the kind
    /// of wake.
    ///
    /// This counts tick **starts**, not completions: the record is written before
    /// any LLM call, so a run killed mid-flight still appears here. That is
    /// deliberate — a completion count drops exactly the runs an alarm exists to
    /// notice — but it makes this series incomparable with any figure derived by
    /// counting `done:` lines in the distiller log.
    pub ticks_last_24h: i64,
    pub max_tick_age_secs: Option<i64>,
    /// Identity next to liveness (#40). A release deploys in two halves through
    /// different hands — a hub swap and a distiller reinstall — and this is the
    /// one read that says whether both landed: `hub_version` ahead of
    /// `last_tick_distiller_version` *after a tick* means the distiller half is
    /// still the old copy. Reported, never alerted on.
    pub hub_version: &'static str,
    /// The release version the last-ticking distiller's script was cut at.
    /// `null` until a tick arrives from a distiller that announces itself.
    pub last_tick_distiller_version: Option<String>,
    /// Git blob id of the distiller file that actually ran — equal to
    /// `git rev-parse <rev>:scripts/cchv-distill.py` for the revision the
    /// installed copy matches, so a reader with the repo can name the commit the
    /// copy came from, or show that it matches none.
    pub last_tick_distiller_blob: Option<String>,
    pub groups: Vec<JournalStaleGroup>,
}

/// The most recent row of `distiller_ticks`, plus how many landed in the last
/// 24 h. One round trip; both halves come off `distiller_ticks_tick_at_idx`.
#[derive(Debug, FromRow)]
struct TickSummary {
    last_tick_at: Option<DateTime<Utc>>,
    last_tick_mode: Option<String>,
    last_tick_groups_pending: Option<i32>,
    last_tick_distiller_version: Option<String>,
    last_tick_distiller_blob: Option<String>,
    ticks_last_24h: i64,
}

/// The inclusive lower bound of the evaluated window: `within_days` before the
/// **current logical day**, not before the current calendar date.
///
/// The anchor is the whole point. This check exists to page when the distiller
/// is behind, so its window must be the window the distiller's forward tick
/// actually covers — and the distiller measures its `--horizon-days` from the
/// same logical day (`scripts/cchv-distill.py::journal_today`, asserted against
/// this function in `scripts/test_cchv_distill.py`). Anchoring one of them on
/// the calendar date instead puts the two out of step between 00:00 and
/// `DAY_START_HOUR` UTC every night: the check counts a day the tick will never
/// pick up, so it reports stale for work nothing will ever do. Measured on m4m
/// 2026-08-21 at 00:46Z, where 6 groups dated 2026-08-13 were counted stale
/// while the tick's own bound was 2026-08-14.
fn horizon_from(now: DateTime<Utc>, day_start_hour: i32, within_days: i32) -> NaiveDate {
    (now - chrono::Duration::hours(i64::from(day_start_hour))).date_naive()
        - chrono::Duration::days(i64::from(within_days))
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
///
/// **A day close is never a net drain of this check, so it has no wall-clock
/// recovery time.** Grace runs from each group's `latest_arrival`, not from the
/// close, so anything that landed more than `grace_secs` before its day closed is
/// stale the instant the day closes. Measured on the live archive 2026-08-21: the
/// 04:00Z roll retired 6 stranded groups and admitted a newly closed day carrying
/// 20, twelve of them already past grace on arrival. Only a distiller tick clears
/// this; the clock never does. Two separate derivations have now predicted a
/// clear-by time from this endpoint and been wrong — do not make a third.
///
/// The tick fields exist for the other half of that mistake. A 503 with a group
/// list says work is undone; it cannot say whether a distiller is chewing through
/// it or has not run at all, because an idle tick writes nothing here. Read
/// together: stale + recent tick is a real stall, stale + no tick is a scheduler
/// or host problem, and the difference decides who gets paged.
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
    // `map` before `parse_positive`, not `parse_positive`'s own default path:
    // absent must stay `None` (report, never alert), so there is no default to
    // fall back to and the `0` below is unreachable.
    let max_tick_age_secs = params
        .max_tick_age_secs
        .as_deref()
        .map(|raw| parse_positive(Some(raw), "max_tick_age_secs", 0))
        .transpose()?;
    let day_start_hour = crate::journal::DAY_START_HOUR;

    // Runtime query (not `query!`): the offline gate has no `.sqlx` metadata for
    // new statements — same reason every query in `journal.rs` is runtime.
    //
    // The fold comes from `journal::SESSION_DAYS_CTE`, not from a copy: this
    // check exists to page when the distiller is behind, so it has to agree with
    // the distiller's own work list about which sessions belong to a day. It
    // used to hold its own transcription of the fold expression, with a comment
    // asking the reader to keep them in step.
    //
    // The CTE's `$2` (inclusive `from` date) is the horizon bound, pushed onto
    // `messages."timestamp"` so this scans the last `within_days` of the archive
    // rather than all of it. It is computed here rather than from `now()` in
    // SQL because the CTE takes a date: a horizon bound is a filter, not a
    // correctness boundary, so app/DB clock skew of much less than a day cannot
    // change the verdict. The closed-day bound below stays on the database's
    // `now()`, where it does matter.
    //
    // `sess_win` bounds the days; `grp` carries the per-session ingest xids for
    // the dirty check and rolls up the arrival. A group is pending when it has no
    // journal row, or its stored session set has drifted from the computed one,
    // or a session's ingest xid is invisible in the row's snapshot (committed
    // after the entry was generated) — commit-order exact, identical to
    // `journal::pending`.
    //
    // There is no `arrivals` CTE any more. It re-joined `messages` purely to get
    // `max(created_at)`, which meant this endpoint scanned the window twice:
    // 6.9 s against 3.7 s for the single-pass form, measured on the live archive.
    // At 6.9 s it intermittently timed out at the proxy and answered 502 — a
    // health check that flaps is worse than a slow one, because the flap is what
    // gets investigated. `SESSION_DAYS_CTE` now carries the arrival, which is
    // both free there and the more correct number (see its doc comment).
    let horizon_from = horizon_from(Utc::now(), day_start_hour, within_days);
    // Raised `work_mem`, scoped to this statement. The fold spills ~35 MB per process
    // at the 4 MB default and this is the whole of #36's fix; the rationale, the
    // measurements and why it is not a covering index live on
    // `journal::JOURNAL_FOLD_WORK_MEM`.
    let mut tx = crate::journal::begin_fold_tx(&state.pool).await?;
    let rows = sqlx::query_as::<_, JournalGroupRow>(&format!(
        r#"
        WITH {session_days},
        sess_win AS (
            SELECT d.session_id     AS session_id,
                   d.entry_date     AS entry_date,
                   p.project_path   AS project_path,
                   d.latest_arrival AS latest_arrival
            FROM msg_days d
            JOIN projects p ON d.project_id = p.id
            WHERE d.entry_date
                    < ((now() - make_interval(hours => $1::int)) AT TIME ZONE 'UTC')::date
        ),
        grp AS (
            SELECT entry_date, project_path,
                   array_agg(session_id ORDER BY session_id) AS session_ids,
                   max(latest_arrival)                       AS latest_arrival
            FROM sess_win
            GROUP BY entry_date, project_path
        )
        SELECT g.entry_date, g.project_path, g.latest_arrival
        FROM grp g
        LEFT JOIN journal_entries j
            ON j.entry_date = g.entry_date AND j.project_path = g.project_path
        WHERE j.id IS NULL
           OR j.session_ids IS DISTINCT FROM g.session_ids
           OR EXISTS (
                SELECT 1
                FROM messages m
                WHERE m.session_id = ANY(g.session_ids)
                  AND m."timestamp" >= ((g.entry_date
                        + make_interval(hours => $1::int)) AT TIME ZONE 'UTC')
                  AND m."timestamp" <  ((g.entry_date + 1
                        + make_interval(hours => $1::int)) AT TIME ZONE 'UTC')
                  AND m.ingest_xid IS NOT NULL
                  AND NOT pg_visible_in_snapshot(m.ingest_xid, j.generated_snapshot))
        ORDER BY g.entry_date DESC, g.project_path DESC
        "#,
        session_days = crate::journal::SESSION_DAYS_CTE,
    ))
    .bind(day_start_hour)
    .bind(horizon_from)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    // Runtime query for the same reason as the one above. Both halves read
    // `distiller_ticks_tick_at_idx`: one backwards probe for the newest row, one
    // range scan for the count.
    //
    // The constant base row with a LATERAL join, rather than three scalar
    // subqueries, is what makes "no tick has ever been recorded" a row of nulls
    // instead of no row at all — the empty-table case is the one this endpoint
    // most needs to be able to answer.
    let ticks = sqlx::query_as::<_, TickSummary>(
        r"
        SELECT t.tick_at           AS last_tick_at,
               t.mode              AS last_tick_mode,
               t.groups_pending    AS last_tick_groups_pending,
               t.distiller_version AS last_tick_distiller_version,
               t.distiller_blob    AS last_tick_distiller_blob,
               (SELECT count(*) FROM distiller_ticks
                 WHERE tick_at > now() - interval '24 hours')::bigint
                   AS ticks_last_24h
        FROM (SELECT 1) AS base
        LEFT JOIN LATERAL (
            SELECT tick_at, mode, groups_pending, distiller_version, distiller_blob
            FROM distiller_ticks
            ORDER BY tick_at DESC
            LIMIT 1
        ) t ON true
        ",
    )
    .fetch_one(&state.pool)
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

    let last_tick_age_secs = ticks.last_tick_at.map(|t| (now - t).num_seconds());
    // Only a supplied threshold can make tick age a verdict. Absent, every
    // branch below is byte-identical to what this endpoint returned before the
    // fields existed — an existing monitor cannot start seeing a new status.
    let tick_overdue = max_tick_age_secs.is_some_and(|limit| match last_tick_age_secs {
        // Never having ticked is the strongest form of overdue, not an unknown:
        // a monitor that asked for a tick-age budget is asking to be told.
        None => true,
        Some(age) => age > limit,
    });

    // `no_tick` outranks `stale`: when both hold, the absent tick is the cause
    // and the undrained groups are its symptom.
    let status = if tick_overdue {
        "no_tick"
    } else if any_stale {
        "stale"
    } else {
        "ok"
    };
    let code = if status == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    Ok((
        code,
        Json(JournalHealthResponse {
            status,
            grace_secs,
            within_days,
            last_tick_at: ticks.last_tick_at,
            last_tick_age_secs,
            last_tick_mode: ticks.last_tick_mode,
            last_tick_groups_pending: ticks.last_tick_groups_pending,
            ticks_last_24h: ticks.ticks_last_24h,
            max_tick_age_secs,
            hub_version: HUB_VERSION,
            last_tick_distiller_version: ticks.last_tick_distiller_version,
            last_tick_distiller_blob: ticks.last_tick_distiller_blob,
            groups,
        }),
    ))
}

// ---------------------------------------------------------------------------
// GET /v1/healthz/stats
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct StatsHealthParams {
    /// Age past which the mirror counts as stale. Defaults to the mirror's
    /// configured `stale_after_secs`. Raw string for the same reason as the
    /// checks above — see [`parse_positive`].
    pub stale_after_secs: Option<String>,
    /// Rows the mirror may trail Postgres by before the check fails. **Absent
    /// by default, and then lag is reported but never alerts** — under live
    /// ingest a healthy mirror always trails by whatever arrived since its last
    /// refresh, so any default here would be a guess about ingest rate dressed
    /// up as a health rule. Setting it keeps that policy in the Gatus check,
    /// where it can be tuned without redeploying the hub (same reasoning as
    /// `healthz_ingest`'s `exclude`).
    pub max_lag_rows: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StatsHealthResponse {
    /// `ok` · `stale` (refreshed too long ago) · `lagging` (missing rows past
    /// `max_lag_rows`) · `warming` (building, no data yet) · `unavailable` (the
    /// mirror file could not be opened at all).
    pub status: &'static str,
    pub ready: bool,
    pub stale_after_secs: i64,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub age_seconds: Option<i64>,
    /// Highest `messages.id` in the mirror.
    pub mirror_max_id: Option<i64>,
    /// Highest `messages.id` in Postgres. `null` when Postgres is unreachable —
    /// which `/v1/healthz` already alerts on, so it is not double-counted here.
    pub postgres_max_id: Option<i64>,
    /// How many ids the mirror trails Postgres by. **This is why age alone is
    /// not enough**: a refresher that runs on schedule but silently steps over
    /// rows keeps the age at zero while the lag grows without bound.
    pub lag_rows: Option<i64>,
    pub max_lag_rows: Option<i64>,
}

/// Statistics-mirror health. Unauthenticated and Gatus-consumable (status code
/// plus a flat body), mirroring the shape of the two checks above.
///
/// Deliberately reports both halves of "is this mirror trustworthy":
/// **recency** (when it last refreshed) and **completeness** (how far its
/// watermark trails Postgres). Silent incompleteness is the one genuinely new
/// failure mode this change introduces — a refresh that skipped rows leaves no
/// trace in the age — and the lag is a two-primary-key lookup, so there is no
/// reason not to answer it.
pub async fn healthz_stats(
    State(state): State<AppState>,
    Query(params): Query<StatsHealthParams>,
) -> Result<(StatusCode, Json<StatsHealthResponse>), HubError> {
    let configured = state
        .mirror
        .as_ref()
        .map_or_else(default_mirror_stale_after, |m| m.stale_after_secs());
    let stale_after_secs = parse_positive(
        params.stale_after_secs.as_deref(),
        "stale_after_secs",
        i64::try_from(configured).unwrap_or(i64::MAX),
    )?;
    let max_lag_rows = params
        .max_lag_rows
        .as_deref()
        .map(|raw| parse_non_negative(raw, "max_lag_rows"))
        .transpose()?;

    let Some(mirror) = state.mirror.as_ref() else {
        // No mirror file at all: a disk-level fault, reported distinctly from a
        // build in progress because the two need different operator responses.
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(StatsHealthResponse {
                status: "unavailable",
                ready: false,
                stale_after_secs,
                refreshed_at: None,
                age_seconds: None,
                mirror_max_id: None,
                postgres_max_id: None,
                lag_rows: None,
                max_lag_rows,
            }),
        ));
    };

    // Runtime query, not `query!`: a PK lookup needs no offline metadata and the
    // CI gate builds with `SQLX_OFFLINE` (see the note atop `journal.rs`).
    let postgres_max_id: Option<i64> =
        match sqlx::query_scalar::<_, i64>("SELECT coalesce(max(id), 0) FROM messages")
            .fetch_one(&state.pool)
            .await
        {
            Ok(v) => Some(v),
            Err(e) => {
                // Not this check's verdict to make: `/v1/healthz` owns database
                // reachability, and reporting a null lag is more honest than
                // reporting a mirror problem that is really a Postgres problem.
                tracing::warn!(error = %e, "healthz/stats could not read the Postgres watermark");
                None
            }
        };

    let state_now = mirror.state();
    let (ready, refreshed_at, age_seconds, mirror_max_id) = match state_now {
        MirrorState::Ready {
            max_id,
            refreshed_at,
        } => (true, Some(refreshed_at), state_now.age_secs(), Some(max_id)),
        MirrorState::Warming => (false, None, None, None),
    };
    let lag_rows = mirror_max_id.zip(postgres_max_id).map(|(m, p)| p - m);

    let status = if !ready {
        "warming"
    } else if age_seconds.is_some_and(|a| a > stale_after_secs) {
        "stale"
    } else if let (Some(limit), Some(lag)) = (max_lag_rows, lag_rows) {
        if lag > limit {
            "lagging"
        } else {
            "ok"
        }
    } else {
        "ok"
    };
    let code = if status == "ok" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    Ok((
        code,
        Json(StatsHealthResponse {
            status,
            ready,
            stale_after_secs,
            refreshed_at,
            age_seconds,
            mirror_max_id,
            postgres_max_id,
            lag_rows,
            max_lag_rows,
        }),
    ))
}

/// Like [`parse_positive`], but zero is a meaningful value here:
/// `max_lag_rows=0` says "any row the mirror is missing is an alert", which is
/// the strictest sensible policy and not a typo. Rejecting it would leave 1 as
/// the tightest expressible budget for no reason.
fn parse_non_negative(raw: &str, name: &str) -> Result<i64, HubError> {
    match raw.parse::<i64>() {
        Ok(v) if v >= 0 => Ok(v),
        _ => Err(HubError::BadRequest(format!(
            "{name} must be a non-negative integer, got {raw:?}"
        ))),
    }
}

/// Fallback threshold when there is no mirror to read the configured one from.
/// Only reachable in the `unavailable` branch, which is a 503 regardless.
fn default_mirror_stale_after() -> u64 {
    crate::config::MirrorConfig::default().stale_after_secs
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

    /// Between midnight and the fold hour, the logical day is still yesterday —
    /// so the window reaches one day further back than a calendar-anchored bound
    /// would. This is the half that agrees with the distiller's forward tick;
    /// the day it drops is the day the tick has also stopped covering.
    #[test]
    fn horizon_is_anchored_on_the_logical_day() {
        let before_fold = "2026-08-21T00:46:00Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(
            horizon_from(before_fold, 4, 7),
            NaiveDate::from_ymd_opt(2026, 8, 13).unwrap(),
            "00:46Z is still the 20th logically, so the bound is the 20th − 7"
        );

        let after_fold = "2026-08-21T04:00:00Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(
            horizon_from(after_fold, 4, 7),
            NaiveDate::from_ymd_opt(2026, 8, 14).unwrap(),
            "the fold hour opens the 21st, and the window slides with it"
        );

        // One second earlier is still the previous logical day: the boundary is
        // half-open at `DAY_START_HOUR`, like `journal::day_bounds`.
        let edge = "2026-08-21T03:59:59Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(
            horizon_from(edge, 4, 7),
            NaiveDate::from_ymd_opt(2026, 8, 13).unwrap()
        );
    }

    #[test]
    fn horizon_scales_with_within_days_and_fold_hour() {
        let now = "2026-08-21T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        assert_eq!(
            horizon_from(now, 4, 1),
            NaiveDate::from_ymd_opt(2026, 8, 20).unwrap()
        );
        // A zero fold hour degenerates to the calendar date — the behaviour this
        // function exists to *not* have at the default hour.
        assert_eq!(
            horizon_from("2026-08-21T00:46:00Z".parse().unwrap(), 0, 7),
            NaiveDate::from_ymd_opt(2026, 8, 14).unwrap()
        );
    }

    /// `max_tick_age_secs` goes through `parse_positive` like the other two, so
    /// a typo in a monitor's URL is a 400 naming the parameter rather than a
    /// silently-ignored threshold that never fires.
    #[test]
    fn max_tick_age_secs_is_validated_like_the_other_params() {
        assert_eq!(
            parse_positive(Some("3600"), "max_tick_age_secs", 0).unwrap(),
            3600
        );
        for bad in ["0", "-1", "abc", ""] {
            let err = parse_positive(Some(bad), "max_tick_age_secs", 0).unwrap_err();
            assert!(matches!(err, HubError::BadRequest(m) if m.contains("max_tick_age_secs")));
        }
    }

    #[test]
    fn parse_non_negative_accepts_zero_but_not_below() {
        assert_eq!(parse_non_negative("0", "max_lag_rows").unwrap(), 0);
        assert_eq!(parse_non_negative("500", "max_lag_rows").unwrap(), 500);
        for bad in ["-1", "abc", ""] {
            let err = parse_non_negative(bad, "max_lag_rows").unwrap_err();
            assert!(matches!(err, HubError::BadRequest(m) if m.contains("max_lag_rows")));
        }
    }
}
