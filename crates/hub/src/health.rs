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
}

#[derive(Debug, Serialize)]
pub struct IngestMachineHealth {
    pub machine_id: Uuid,
    pub hostname: String,
    pub last_seen: DateTime<Utc>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub stale: bool,
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

/// Staleness is judged on `machines.last_seen` alone (the daemon's
/// ingest-upsert heartbeat) — never on message recency, so an idle machine
/// with no new coding sessions doesn't page anyone.
pub async fn healthz_ingest(
    State(state): State<AppState>,
    Query(params): Query<IngestHealthParams>,
) -> Result<(StatusCode, Json<IngestHealthResponse>), HubError> {
    let stale_after_secs = parse_stale_after_secs(params.stale_after_secs.as_deref())?;

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
            any_stale |= stale;
            IngestMachineHealth {
                machine_id: r.machine_id,
                hostname: r.hostname,
                last_seen: r.last_seen,
                last_message_at: r.last_message_at,
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
        Json(IngestHealthResponse {
            status: if any_stale { "stale" } else { "ok" },
            stale_after_secs,
            machines,
        }),
    ))
}
