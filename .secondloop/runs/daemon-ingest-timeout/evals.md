# Eval rubric — Sync-daemon ingest request timeout + per-batch deadline

Feature: fix the silent-wedge failure mode behind Gitea issue #2 (live
incident 2026-07-04, `ac-mbm5`) — the sync daemon's ingest HTTP client
(`ReqwestHubClient` in `crates/sync-daemon/src/client.rs`) is built from
`reqwest::Client::new()`, which has **no timeout of any kind**. A request that
straddled a laptop sleep cycle blocked forever inside `send()`, wedging the
whole `run()` loop for 12.5 hours with no sync passes, no log output, and a
frozen hub `last_seen` — recovery required a manual `launchctl kickstart`.

Two independent, layered fixes are in scope:

1. A per-request timeout on the `reqwest::Client` itself (default 30s,
   `CCHV_INGEST_TIMEOUT_SECS` env override). A timed-out request is a
   transport error and must flow through the *existing* retry/backoff loop —
   it must not become a single-shot failure.
2. A per-batch deadline (`tokio::time::timeout`) around each
   `client.ingest(&batch)` call inside `sync::run_once`, default 600s
   (`CCHV_INGEST_DEADLINE_SECS` env override). This is defense-in-depth: it
   bounds a sync pass even if a `HubClient` implementation hangs for *any*
   reason — the trait is the extension point, and the daemon must never again
   trust an ingest future to resolve. On deadline expiry the batch is counted
   in `SyncStats::errors`, logged via `tracing::warn!`, and **not**
   checkpointed, so it is retried from the same position next pass
   (at-least-once delivery; the hub dedupes).

All four acceptance criteria are backend-observable Rust behavior of
`sync_daemon::client::ReqwestHubClient` and `sync_daemon::sync::run_once` —
all T2. There is no frontend-observable surface for this feature (no UI), so
there is no T1 file.

Eval file: `crates/loop-evals/tests/daemon-ingest-timeout_eval.rs` (4
`#[tokio::test]`s, each `#[serial]` because they mutate process-global env
vars and/or `$HOME`).

## How each eval fails cleanly against the unmodified crate

Every eval wraps the call under test in an *outer* `tokio::time::timeout`
(120s for the AC1/AC2 client-level evals, 60s for the AC3/AC4 sync-level
evals). Against the current code — no request timeout, no batch deadline —
the wrapped call hangs forever, so the outer timeout fires and the eval fails
with a clear panic message instead of hanging the test run. This was verified
by actually running each eval against the unmodified tree: AC3/AC4 fail at
exactly 60.01s with `Elapsed(())`; AC1/AC2 fail at exactly 120.01s.
Once the fix lands, both env vars are set to small values (1–2s) in the
evals, so the fixed code returns in ~10–30s (a few retries + backoff), well
inside each outer bound.

## Criteria

### AC1 — timed-out request returns `Err`, doesn't hang forever (T2)
With `CCHV_INGEST_TIMEOUT_SECS=1`, `ReqwestHubClient::ingest` called against a
black-hole TCP server (accepts every connection, never writes a response)
returns `Err` within the 120s outer bound.
Eval: `ac1_timed_out_request_returns_err_instead_of_hanging_forever`.

### AC2 — timed-out requests still go through the existing retry loop (T2)
In the same black-hole scenario, the server must have accepted **at least 2**
connections by the time `ingest` returns `Err` — proving the per-request
timeout turns a hang into a *retryable* transport error (consumed by the
existing `max_retries`/backoff loop in `client.rs`), not a single-shot
failure.
Eval: `ac2_timed_out_requests_go_through_the_existing_retry_loop`.

### AC3 — per-batch deadline bounds a hanging `HubClient`, counts an error (T2)
With `CCHV_INGEST_DEADLINE_SECS=2`, `sync::run_once` called with a `HubClient`
whose `ingest` is `std::future::pending().await` (never resolves) and a
`$HOME` fixture containing one syncable Claude session (2-message JSONL,
shape copied from `crates/sync-daemon/tests/sync_test.rs`) returns within the
60s outer bound and reports `SyncStats::errors >= 1`.
Eval: `ac3_deadline_aborts_hanging_ingest_and_counts_an_error`.

### AC4 — the aborted batch isn't checkpointed, so it's redelivered (T2)
After that same deadline-aborted `run_once` pass (`errors >= 1`, checkpoint
left empty), a second `run_once` over the same checkpoint state — this time
with a normal recording `HubClient` mock — delivers the fixture session's
messages in full. The aborted batch was never marked synced, so no data is
lost; at-least-once delivery holds.
Eval: `ac4_aborted_batch_is_not_checkpointed_and_redelivers_next_pass`.

## Non-goals (not evaluated, per spec)

File-watching, launchd/watchdog process supervision, `daemon.toml` schema
changes, and hub-side changes are explicitly out of scope for this feature
and have no evals here.
