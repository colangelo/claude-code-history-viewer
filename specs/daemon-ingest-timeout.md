# Sync-daemon: ingest request timeout + per-batch deadline (no more silent wedge)

## Description

Live incident (2026-07-04, ac-mbm5; Gitea issue #2): the sync daemon's ingest
HTTP client is `reqwest::Client::new()` — **no timeout of any kind**
(`crates/sync-daemon/src/client.rs`). An ingest request that straddled a
laptop sleep cycle blocked forever inside `send()`, wedging the whole
`run()` loop for 12.5 hours while the process stayed alive: no sync passes,
no log output, hub `last_seen` frozen, machine flagged stale. Recovery
required a manual `launchctl kickstart`.

Two independent, layered fixes in `crates/sync-daemon`:

1. **Per-request timeout** (`client.rs`): build the `reqwest::Client` with a
   total request timeout. Default **30 seconds**; overridable via the
   `CCHV_INGEST_TIMEOUT_SECS` env var (positive integer seconds; unset or
   invalid → default). A timed-out request is a transport error and goes
   through the **existing retry/backoff loop** (it must NOT become a
   single-shot failure — the hub being briefly down should still be retried
   up to `max_retries`).

2. **Per-batch deadline at the sync layer** (`sync.rs`): wrap each
   `client.ingest(&batch)` call in `run_once` with a hard deadline
   (`tokio::time::timeout`). Default **600 seconds**; overridable via
   `CCHV_INGEST_DEADLINE_SECS` (same parsing rules). This is
   defense-in-depth: it bounds a sync pass even if a `HubClient`
   implementation hangs for ANY reason (the trait is the extension point —
   the daemon must never again trust an ingest future to resolve). On
   deadline expiry: count it in `stats.errors`, log a `tracing::warn!`, and
   **do not record the file's checkpoint** — the batch must be retried from
   the same position on the next pass (at-least-once delivery preserved;
   the hub dedupes).

Both env vars are read where the value is used (construction time is fine);
read via `std::env::var` — tests mutate env process-globally, which is safe
under the repo's `--test-threads=1` convention. Keep clippy pedantic clean.
Do not change `ReqwestHubClient::new`'s signature or the `HubClient` trait —
existing callers and tests must keep compiling.

Non-goals (do NOT implement): file-watching, launchd/watchdog process
supervision, changes to `daemon.toml` config schema, hub-side changes.

Eval mechanics (T2, `loop-evals` crate, all runtime assertions through the
EXISTING public surface — `sync_daemon::client::{HubClient, ReqwestHubClient}`,
`sync_daemon::sync::run_once`, `sync_daemon::identity::Identity`,
`sync_daemon::checkpoint::Checkpoint` are all pub today):

- Black-hole hub: bind a `tokio::net::TcpListener` on `127.0.0.1:0`, accept
  connections and hold the streams without ever writing a response. Count
  accepted connections behind an `Arc<Mutex<_>>`.
- Hanging client: a local `struct` implementing `HubClient` whose `ingest`
  is `std::future::pending().await` — never resolves.
- Session fixture for `run_once`: point `$HOME` at a tempdir containing
  `.claude/projects/<dir>/<uuid>.jsonl` with 2–3 valid user/assistant lines —
  copy the fixture shape from `crates/sync-daemon/tests/sync_test.rs`. Env
  and `$HOME` are process-global: mark every eval `#[serial]` (serial_test
  is available in loop-evals).
- Every eval wraps the operation under test in an OUTER
  `tokio::time::timeout` and fails (does not hang) if it fires — that outer
  bound is what makes each eval fail against the unmodified crate, where the
  operation blocks forever. Generous outer bounds (60–120 s) so the fix
  passes with margin; post-fix runtimes with 1–2 s env-var settings are
  ~10–30 s including retries and backoff.

## Acceptance Criteria

- (T2) With `CCHV_INGEST_TIMEOUT_SECS=1`, `ReqwestHubClient::ingest` pointed at a black-hole TCP server (accepts, never responds) returns `Err` within an outer 120-second bound instead of hanging forever.
- (T2) In that same black-hole scenario the server has accepted at least 2 connections by the time `ingest` returns `Err` — proving timed-out requests went through the existing retry loop rather than failing single-shot.
- (T2) With `CCHV_INGEST_DEADLINE_SECS=2`, `sync::run_once` called with a hanging `HubClient` (`ingest` never resolves) and a `$HOME` fixture containing one syncable Claude session returns within an outer 60-second bound and reports `errors >= 1` in its `SyncStats`.
- (T2) After that deadline-aborted pass, a second `run_once` over the same checkpoint with a recording mock client delivers the fixture session's messages — the aborted batch was not checkpointed as synced, so no data is lost.
