# hub-mirror-refresh-timeout — Tasks

## 1. Bound the refresh attempt

- [x] 1.1 Add `refresh_timeout_secs` (default 900) and
      `cold_build_timeout_secs` (default 21600) to `MirrorConfig` in
      `crates/hub/src/config.rs`, with `serde` defaults and `Default` wiring,
      documenting *why* each ceiling exists rather than what it is
- [x] 1.2 Add `RefreshOutcome::TimedOut` in `crates/hub/src/mirror.rs` and
      handle it in `RefreshOutcome::report()` (no report — nothing ran)
- [x] 1.3 Add `Mirror::refresh_bounded(&self, pg, budget)` wrapping `refresh`
      in `tokio::time::timeout`, mapping elapse to `Ok(TimedOut)` not `Err`
      (design D3), and documenting the production wedge it exists to prevent
- [x] 1.4 Switch `run_refresher` to `refresh_bounded`, selecting the budget by
      `mirror.is_empty()` (cold build vs incremental, design D2) and logging a
      timeout distinctly from a failure

## 2. Regression test

- [x] 2.1 Add a blackhole-listener helper to
      `crates/hub/tests/mirror_lifecycle_test.rs` — accepts, never speaks,
      never closes — so the hang is deterministic and needs no Postgres
- [x] 2.2 Test that a hung refresh returns `TimedOut` within the budget, that a
      **second** attempt also returns `TimedOut` rather than `Skipped` (the
      latch was released — design D4), and that `run_refresher` survives it
- [x] 2.3 Confirm the existing `repeated_refresh_failures_leave_the_mirror_serving`
      and `a_refresh_arriving_during_another_is_skipped` tests still pass —
      the `Err` and `Skipped` paths must be unchanged

## 3. Verify

- [x] 3.1 `cargo test -p hub --test mirror_lifecycle_test` (the new test needs
      no database; the neighbours need `TEST_DATABASE_URL`)
- [x] 3.2 `cargo clippy --all-targets --all-features -- -D warnings` and
      `cargo fmt --all -- --check`
- [ ] 3.3 Reply to infra (ac/infra#93) with the root cause, the fix, and the
      immediate mitigation (a hub restart clears the wedge; the durable fix
      ships in the next `cchv-v*` hub binary)
