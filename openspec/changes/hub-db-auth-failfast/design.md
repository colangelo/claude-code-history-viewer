# Design — hub fail-fast on a rotated Postgres credential

## Context

The hub is the only component holding Postgres credentials. `hub::run()` builds a
`PgPool` from `config.database_url` at startup and never revisits that value; the
process then runs for weeks. As of 2026-07-25 the password behind it is owned and
rotated by OpenBao (`database/static-creds/cchv-svc`, 30-day period), so that value now
has an expiry the hub knows nothing about.

Two facts shape the whole design:

- **The recovery machinery already exists and is unused.** `dev.cchv.hub` on m4m runs
  `cchv-launch hub` under `KeepAlive=true` with `ThrottleInterval=300`. Any exit is
  already followed by a relaunch that re-reads bao and renders a fresh config
  (`bao_static` → `database/static-creds/cchv-svc`, landed in `4b0de0ab` and live). The
  hub simply never exits, so that path is never taken.
- **A restart is close to free.** Ingest is at-least-once with retry/backoff over a
  crash-safe checkpoint, the embedder is lazily loaded, the embedding sweep is
  incremental, and the journal distiller catches up on its next pass. Readers see a few
  seconds of 502. This is why "restart" is an acceptable primitive here and in-process
  credential renewal is not warranted.

Constraint carried over from issue #17: a transient MagicDNS flake against pg1 has
previously 500'd reads, and the mitigation (`min_connections(2)`, `test_before_acquire`,
a 5 s `acquire_timeout`) is deliberate. Nothing here may turn that flake into an exit.

## Goals / Non-Goals

**Goals**

- A rotated password heals with no human involved, well inside the rotation period.
- Detect before the hub degrades, not after.
- Make it impossible for database or network unavailability to trigger an exit.
- Keep the change small and legible: one module, a few lines in `run()`.

**Non-Goals**

- In-process credential renewal or pool rebuilding. Explicitly rejected — the supervisor
  does this correctly already, and a re-resolving pool would duplicate `cchv-launch`.
- Graceful connection draining on exit. The hub is already failing when this fires, and
  the daemon's at-least-once retry covers in-flight ingest.
- Reacting to any failure other than authentication. A down pg1 is not fixed by
  restarting the hub.
- New configuration surface. `ThrottleInterval` stays the only operational dial.

## Decisions

### Probe with a fresh connection, not one borrowed from the pool

Each probe opens its own short-lived connection and runs `SELECT 1`.

*Why:* the pool masks exactly the condition we care about. After a rotation, pooled
connections keep authenticating until they age out (sqlx's `max_lifetime` defaults to 30
minutes), so a pool-based probe stays green while requests that need a *new* connection
fail. Detection would lag by up to half an hour of intermittent 500s. A fresh connection
sees the rotation on the next tick.

*Alternative considered — probe the pool with `SELECT 1` (what `/v1/healthz` does):*
cheaper and reuses existing code, and it never exits a hub that can still serve. Rejected
because "can still serve" is temporary and misleading here: once the password has
changed, the hub **must** restart to be correct, and doing it promptly converts ~30
minutes of degrading service into a few seconds of downtime.

*Cost accepted:* one extra connect per interval, forever. Negligible against pg1.

### Interval 30 s, strike limit 3

Exit roughly 90 seconds after a rotation; launchd heals it inside two minutes, far below
the 300 s `ThrottleInterval` so there is no respawn churn.

*Why three:* it rules out a one-off without waiting long. It is a cheap guard rather than
a statistical one — an authentication rejection is already unambiguous, and the
classifier below means a blip cannot accumulate strikes at all.

*Alternatives considered:* 60 s × 5 (≈5 min) buys confirmations against nothing in
particular and sits awkwardly against `ThrottleInterval`; 10 s × 3 (≈30 s) heals fastest
but connects to pg1 every ten seconds forever, which is more churn than the problem
justifies.

### Count only SQLSTATE `28P01`; every other outcome resets the counter

The strike counter increments **only** on a Postgres authentication rejection, and is
reset to zero by success, by any other SQLSTATE, by an I/O or DNS error, by a pool
timeout, and by a probe timeout.

*Why this is safe:* verified in sqlx 0.8.6 — `sqlx-postgres`'s
`is_transient_in_connect_phase` returns true for only `53300` (too many connections) and
`57P03` (cannot connect now). Everything else non-transient is returned from the connect
path as `Error::Database` rather than being retried until the deadline and collapsed into
`PoolTimedOut`. So `28P01` is observable precisely, and the classifier does not have to
infer auth failure from a timeout.

*Alternative considered — also exit after a long period of any DB failure:* rejected. It
re-opens #17, and restarting into a down database accomplishes nothing.

### The classifier takes a SQLSTATE code, not a `sqlx::Error`

The predicate is `is_auth_code(code: Option<&str>) -> bool`, with a one-line adapter that
extracts the code from `sqlx::Error::Database`.

*Why:* this predicate is the entire safety property, so it must be exhaustively
unit-tested — but `PgDatabaseError`'s fields are private and it cannot be constructed in
a test without a live server handshake. Taking a code makes the logic trivially testable
with no database, and confines the untestable part to an adapter too small to hide a bug.

### Exit by returning an error from `run()`, not `process::exit` inside the task

The watchdog signals through an `Arc<Notify>`; `run()` selects over `axum::serve(...)`
and that notification, and the notified arm returns an error. `main()` propagating that
error produces the non-zero exit launchd needs.

*Why:* the exit decision stays in the function that owns the server's lifetime, axum
stops cleanly when the select resolves, and the reason travels through the normal error
path (logged, not swallowed). This mirrors the existing `AppState::embed_nudge` use of
`Notify`.

*Alternatives considered:* `std::process::exit(1)` from inside the spawned task is
shorter but buries a process-lifetime decision in a background job and skips any
unwinding; a panic would satisfy `KeepAlive` too but misrepresents an expected
operational event as a bug and pollutes the crash signal.

### Module shape

A new `crates/hub/src/db_watchdog.rs`, modelled on `embed_sweep`: a `pub async fn`
spawned from `run()` that loops over probe-then-sleep. It takes the database URL, a
`Notify`, an interval, and a limit — it does **not** take `AppState`, so it has no
dependency on the pool, the tokens, or the embedder, and can be reasoned about alone.

## Risks / Trade-offs

- **A non-rotation `28P01` crash-loops the hub** (role dropped, `pg_hba` change,
  password reset out of band) → the loop is bounded by `ThrottleInterval` to one attempt
  per 5 minutes, Gatus's `cchv-hub` check goes red, and the launcher's own WARN names a
  stale cached credential as a candidate cause. This is the correct behaviour: such a
  state genuinely needs a human, and a hub quietly serving 500s is worse than one
  visibly restarting.
- **Bao unreachable at the moment of relaunch after a rotation** → `cchv-launch` falls
  back to its last-known-good render, which is now stale, so the hub exits again. Bounded
  by `ThrottleInterval`, self-healing the moment bao returns, and already documented in
  `docs/archive/deployment.md` §3b. Accepted knowingly: the alternative (fail closed
  instead of using the cache) loses the common case where the cache is still valid.
- **The assumption that a rotation surfaces as `28P01`** → pinned by an integration test
  that authenticates against a real Postgres with a deliberately wrong password and
  asserts the classification, so a future sqlx upgrade that changes the mapping fails CI
  instead of failing in production a month later.
- **Watchdog holds the database URL in memory** → same exposure as the pool it sits
  beside; the design forbids logging it, and the spec carries that as a requirement.
- **One more always-on background task** → it shares the existing runtime and does
  nothing but sleep between probes.

## Migration Plan

1. Land on `main` via the worktree branch (`feature/hub-db-auth-failfast`).
2. Cut a hub release; the binary is built by CI for the tag.
3. Deploy to m4m as a §2b binary swap (rm-first, fresh inode,
   `codesign --force --sign -`, `bootout` + `bootstrap`, never `kickstart -k`).
   **Relay it as two messages, not one** — per home-network #34, infra's handler hit its
   900 s ceiling on the single-message v0.14.0 deploy.
4. Verify: the hub comes back healthy, and the launch log still shows
   `db password from bao static-creds/cchv-svc`.

*Deadline:* the first automatic rotation is **2026-08-24T13:38:54Z**. Until this ships,
a rotation is a silent outage requiring a manual bounce.

*Rollback:* swap back the `preswap` binary per §2b. The watchdog disappears with it and
behaviour reverts to today's (no exit, 500s until a human bounces). No data, schema, or
config migration is involved, so rollback is complete and instant.

## Open Questions

None blocking implementation. Two items deliberately deferred:

- Should the interval and strike limit become configurable? Only if a non-launchd
  deployment ever needs different values. YAGNI until then.
- The launcher's `kv/infra/cchv/pg1` fallback becomes dead code once infra retires the
  1Password item; removing it is tracked with that retirement, not here.
