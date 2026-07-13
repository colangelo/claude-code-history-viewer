//! `GET /v1/healthz` — unauthenticated liveness + database connectivity check.
//! `GET /v1/healthz/ingest` — unauthenticated per-machine ingest-daemon
//! liveness, so Gatus (HTTP status/body only) can alert on a dead daemon even
//! while `/v1/healthz` stays green.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
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
