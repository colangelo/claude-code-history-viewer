# Hub ingest-freshness endpoint (per-machine daemon liveness for monitoring)

## Description

Tonight's incident class: `/v1/healthz` stayed green while ingestion was dead —
a daemon killed by codesigning on one machine, then 413/NUL rejections blocking
batches on another. ~43k messages were silently stuck. Gatus (the house uptime
monitor) can only evaluate HTTP status/body conditions, so the hub must compute
staleness itself and expose it as an endpoint Gatus can poll.

Add **`GET /v1/healthz/ingest`** to the hub (`crates/hub`):

- **Unauthenticated**, exactly like the existing `/v1/healthz` (the hub is
  tailnet-only; Gatus polls without a bearer token).
- Reports **every machine** in the `machines` table with: `machine_id`,
  `hostname`, `last_seen` (RFC 3339), `last_message_at` (max
  `messages.created_at` for that machine, RFC 3339 or null when the machine
  has no messages), and a boolean `stale`.
- **Staleness is judged on `machines.last_seen` ONLY** — the daemon upserts
  `last_seen = now()` on every ingest (even empty-delta passes touch it via the
  machine upsert), so it is a daemon-liveness heartbeat. `last_message_at` is
  exposed for observability but never alerted on: an idle machine (no new
  coding sessions) must not page anyone.
- Threshold: query param `stale_after_secs`, default **7200** (2× the daemons'
  hourly scan interval). A machine is stale when
  `now() - last_seen > stale_after_secs`. Non-numeric or non-positive values →
  **400**.
- Response: HTTP **200** with `{"status":"ok", ...}` when no machine is stale;
  HTTP **503** with `{"status":"stale", ...}` when at least one is. Body always
  includes `stale_after_secs` (the effective threshold) and the `machines`
  array. An **empty archive (no machines) → 200 "ok"** with an empty array
  (bootstrap must not alarm). Gatus will use `[STATUS] == 200`.

Implementation shape: follow the existing hub module conventions —
`crates/hub/src/health.rs` (or a sibling module) + route in `lib.rs::router`,
sqlx query against `machines` LEFT JOIN a per-machine max of `messages`,
`HubError::BadRequest` for the 400 case. Keep clippy pedantic clean.

Eval mechanics (T2, `loop-evals` crate): spawn the in-process hub per the
RUNBOOK (`hub::MIGRATOR` + `hub::router` on `127.0.0.1:0`), seed machines via
`POST /v1/ingest`. To simulate a dead daemon deterministically, backdating is
the one permitted raw-SQL exception:
`UPDATE machines SET last_seen = now() - interval '3 hours' WHERE machine_id = $1`
on the test pool. Each test uses fresh random machine ids (existing test-db
isolation pattern).

## Acceptance Criteria

- (T2) With one machine ingested and its `last_seen` fresh, `GET /v1/healthz/ingest` returns 200 with `status:"ok"` and a machines array whose entry carries `machine_id`, `hostname`, `last_seen`, `last_message_at`, and `stale:false`.
- (T2) With two machines where one has `last_seen` backdated beyond the threshold, the endpoint returns 503 with `status:"stale"`, the backdated machine `stale:true`, and the fresh machine `stale:false`.
- (T2) `stale_after_secs` is honored: a machine backdated 3 hours is stale at the default (7200s) but not stale with `?stale_after_secs=14400`; a non-numeric or non-positive `stale_after_secs` returns 400.
- (T2) Staleness ignores message recency: a machine whose `last_seen` is fresh but which has zero messages reports `last_message_at:null`, `stale:false`, and the endpoint returns 200.
- (T2) The endpoint answers without any `Authorization` header (same policy as `/v1/healthz`), and with no machines in the archive it returns 200 with `status:"ok"` and an empty machines array.
