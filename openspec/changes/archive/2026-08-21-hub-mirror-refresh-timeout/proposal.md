# hub-mirror-refresh-timeout

## Why

The statistics mirror on m4m stopped refreshing at 2026-08-13 06:31:05Z and was
still frozen 6.4 h later — 417,082 rows behind Postgres — while `/v1/stats/*`
kept answering `200` from it. `/v1/healthz` and `/v1/healthz/ingest` were green
throughout; ingest never missed a beat. Reported by infra (ac/infra#93) from
their Gatus checks, which caught the staleness correctly.

The mirror's degradation contract only covers a refresh that **fails**. This one
never failed — it never *returned*:

- `run_refresher` awaits `refresh()` with no time bound, and sleeps only
  *after* it returns. One attempt that never returns is not a slow tick, it is
  the last tick the process will ever run.
- Nothing else bounds it. `acquire_timeout` (5 s) bounds only *getting* a
  pooled connection, and no statement timeout is set, so a query already in
  flight waits on the socket indefinitely.
- The single-flight latch is held for the whole attempt, so even a manual
  trigger would answer `Skipped`.
- `MirrorState::Ready` is derived from the *file*, not from refresher liveness,
  so the API stayed `ready: true` and served stale analytics rather than erroring.

Measured on m4m while the wedge was live: local port 49234 was `ESTABLISHED` to
pg1:5432 with **no matching backend in `pg_stat_activity`** — the peer was
reaped and no `FIN`/`RST` ever arrived. It was the lowest-numbered of the hub's
six sockets, i.e. the oldest, and the DuckDB WAL was held open with an mtime of
07:08:48Z. The trigger was environmental (m4m's default route is a ProtonVPN
tunnel, so pg1 is relayed via DERP — infra
`debug-logs/2026-08-13-m4m-protonvpn-derp-relay-tailnet-throughput.md`). The
reason it never recovered is ours: only a hub restart could clear it.

## What Changes

- Bound every refresh attempt: `Mirror::refresh_bounded` wraps `refresh` in a
  wall-clock ceiling and returns a new `RefreshOutcome::TimedOut` rather than
  hanging. Cancelling by dropping the future releases the single-flight latch
  through the existing `InFlight` guard, so the *next* tick can run — a timeout
  that freed the loop but leaked the latch would leave every later tick
  `Skipped`, which looks healthy in the logs and leaves the mirror just as frozen.
- `run_refresher` uses it, logging the timeout distinctly from a failure and
  continuing to the next tick.
- Two config knobs: `refresh_timeout_secs` (default 900) and
  `cold_build_timeout_secs` (default 21600). A cold build legitimately takes
  minutes and must not be charged the incremental budget — cancelled on every
  tick it would never finish and `/v1/stats/*` would answer 503 permanently.

Out of scope: making a wedged refresher visible as *unreadiness*. Monitoring
already caught this — `/v1/healthz/stats` reported the staleness exactly as
designed. The defect was that nothing recovered from it, and serving a stale
answer during a refresh fault is the deliberate existing contract.

## Capabilities

### Modified Capabilities

- `hub-stats-mirror`: the "Refresh failures degrade rather than escalate"
  requirement is widened from *failing* to *not completing* — a refresh that
  hangs must be cancelled and retried on the following tick rather than
  stopping the refresher for the life of the process.
