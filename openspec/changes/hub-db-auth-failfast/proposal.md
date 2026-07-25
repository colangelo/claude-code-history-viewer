# Hub fail-fast on a rotated Postgres credential

## Why

The hub's Postgres password is now a credential **OpenBao owns and rotates on its own**
(`database/static-creds/cchv-svc`, `rotation_period=30d`, provisioned 2026-07-25T13:38:54Z
— home-network #31). The hub resolves that password **once, at process start**, and then
runs for weeks. When bao rotates it, the running hub keeps serving on its open pool, then
fails on every new connection as those recycle — and **nothing exits**, so it sits
returning 500s (with `/v1/healthz` reporting 503) until a human bounces it.

The first automatic rotation is **2026-08-24T13:38:54Z**. That is a hard deadline: on
that date, without this change, the archive goes down and stays down until someone
notices. Tracked as cchv Gitea #25.

## What Changes

- The hub gains a **database credential watchdog**: a background task that probes
  Postgres on an interval and, after a small number of *consecutive authentication*
  failures, terminates the process with a non-zero exit.
- A non-zero exit is all that is needed to heal, because the supervision already exists:
  `dev.cchv.hub` on m4m runs `cchv-launch hub` under `KeepAlive=true` with
  `ThrottleInterval=300`, so launchd relaunches it and `cchv-launch` re-reads
  `database/static-creds/cchv-svc`, rendering a fresh config with the new password.
- The probe deliberately opens a **fresh connection** rather than borrowing from the
  pool, so a rotation is detected while pooled connections still work. The hub therefore
  restarts *before* it degrades, instead of after.
- Only SQLSTATE **`28P01`** (`invalid_password`) counts toward the strike total.
  Every other outcome — `Io`, `PoolTimedOut`, probe timeout, any other SQLSTATE —
  **resets** the counter, so a transient MagicDNS or pg1 blip can never exit the hub
  (that is issue #17's territory, and its `min_connections(2)` mitigation stays intact).
- No new configuration surface. Interval and strike limit are constants;
  `ThrottleInterval` in the plist remains the only operational dial.
- Not breaking. No API, schema, or client change; `/v1/healthz` behaviour is untouched.

## Capabilities

### New Capabilities

- `hub-credential-resilience`: how the hub responds when its database credential stops
  being valid — what it detects, what it deliberately ignores, and how it hands recovery
  to its process supervisor rather than attempting in-process credential renewal.

### Modified Capabilities

<!-- None. No existing capability's requirements change: the read/ingest APIs, the
     health endpoints, and the daemon contract all behave exactly as specified today.
     This change adds process-lifecycle behaviour that no current spec covers. -->

## Impact

- **Code**: `crates/hub` — one new module (the watchdog) plus a few lines in
  `hub::run()` to spawn it and to exit when it fires. Nothing else in the crate changes.
- **Operations**: a hub release and a §2b binary swap on m4m
  (`docs/archive/deployment.md`). Per home-network #34, that deploy should be relayed as
  **two messages** rather than one, because infra's relay handler hit its 900 s ceiling
  on the single-message v0.14.0 deploy.
- **Runtime cost**: one short-lived Postgres connection per probe interval, forever.
  Negligible against pg1, and it replaces an unbounded human-response-time outage.
- **Dependencies**: none added. Relies on `sqlx` 0.8 surfacing `28P01` as
  `Error::Database` — verified: sqlx-postgres treats only `53300` and `57P03` as
  transient-in-connect, so a non-transient connect error bubbles up rather than being
  collapsed into `PoolTimedOut`.
- **Prerequisite already landed**: `4b0de0ab` taught `cchv-launch` to read
  `database/static-creds` (`bao_static`, `.data.password`), and it is installed and in
  use live on m4m. Without it a relaunch would re-render the superseded `kv/` mirror,
  and this watchdog would produce a throttled crash loop instead of a recovery.
