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
  `last_seen = now()` on ingest (even empty-delta passes touch it via the
  machine upsert), so it is a daemon-liveness heartbeat. `last_message_at` is
  exposed for observability but never alerted on: an idle machine (no new
  coding sessions) must not page anyone.
- **`last_seen` has 60-second write granularity.** The machine upsert's
  `DO UPDATE` is guarded (`machines.last_seen < now() - interval '60 seconds'`,
  plus any hostname/os change) because an unconditional bump made this 3-row
  table the archive's hottest write — one HOT update per ingest request. So
  `last_seen` may lag a live daemon by up to 60 s; the 7200 s default threshold
  absorbs that with 120× headroom. Any future threshold must stay comfortably
  above the coalescing window.
- Threshold: query param `stale_after_secs`, default **7200** (2× the daemons'
  hourly scan interval). A machine is stale when
  `now() - last_seen > stale_after_secs`. Non-numeric or non-positive values →
  **400**.
- **Alert-set exclusion**: query param `exclude`, a comma-separated list of
  hostnames. Matching is case-insensitive and tolerant of the mDNS `.local`
  suffix (whitespace/empty entries ignored), so `exclude=ac-mbp` matches the
  archive's stored `ac-mbp.local` — the operator need not know the suffix. An
  excluded machine still appears in `machines` with its real `stale` flag and a
  new `excluded:true`, but it **does not count toward the overall stale/503
  verdict**. This is for decommissioning machines whose dead daemon is expected
  (e.g. `ac-mbp`) — masking them here, rather than raising the threshold or
  accepting 503, keeps the check able to still page on a *real* dead daemon on a
  live machine. Keeping it a query param leaves the policy in Gatus's check
  config, so changing the excluded set needs no hub redeploy.
- Response: HTTP **200** with `{"status":"ok", ...}` when no machine is stale;
  HTTP **503** with `{"status":"stale", ...}` when at least one is. Body always
  includes `stale_after_secs` (the effective threshold) and the `machines`
  array. An **empty archive (no machines) → 200 "ok"** with an empty array
  (bootstrap must not alarm) — implement this path, but it is verified by
  review rather than an eval: the shared integration-test database is never
  empty (it accumulates machines from other suites), so no deterministic eval
  exists for it. Gatus will use `[STATUS] == 200`.

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
- (T2) The endpoint answers without any `Authorization` header — no 401/403 — matching the `/v1/healthz` policy.
- (T2) `exclude` drops machines from the alert verdict: a stale machine stored as `host.local` whose bare hostname is passed in `exclude` (case- and `.local`-suffix-insensitively) still reports `stale:true` with `excluded:true`, and excluding every currently-stale hostname flips the endpoint from 503 to 200 `"ok"` at the same threshold.
