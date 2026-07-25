# hub-stats-duckdb-mirror — tasks

This file is the implementation plan. Measurements referenced below are in
`design.md`; do not re-derive them.

## 1. Dependency and groundwork

- [ ] 1.1 Add `duckdb` (bundled) to `crates/hub`. Record the resulting binary size and the `cargo build --release -p hub` wall time before and after — the macos-14 release job has to keep fitting, and this is the one dependency in the change that could break CI rather than tests
- [ ] 1.2 Confirm disk headroom on m4m for the mirror (119 MB at today's scale, growing with the archive) and note the figure for the deploy relay
- [ ] 1.3 Add mirror settings to `HubConfig`: file path (default under the hub config dir), refresh interval, DuckDB `memory_limit`, and the staleness threshold used by the health endpoint. All optional with defaults, so an existing `hub.toml` keeps working untouched

## 2. The mirror module

- [ ] 2.1 Create `crates/hub/src/mirror.rs` owning the file and nothing else: `open_or_create`, `state() -> Warming | Ready { max_id, refreshed_at }`, `refresh() -> RefreshReport`. It MUST NOT know anything about statistics
- [ ] 2.2 Define the mirror schema: the 13 `messages` columns the rollups read plus the derived `usage_row` and `conversational` booleans, `sessions(id, project_id)`, `projects`, `message_tool_uses`, `message_tool_results`
- [ ] 2.3 Implement the full build: pull each table from Postgres and populate the mirror. Measured at 227 s for 2.8M messages, network-bound
- [ ] 2.4 Implement incremental refresh: append rows `WHERE id > max_id`, computing the derived booleans for the new rows only (design D2). Never recompute existing rows
- [ ] 2.5 Guard refresh with single-flight — a tick arriving while one runs is skipped, not queued
- [ ] 2.6 On an unopenable or corrupt mirror, move the file aside under a timestamped name and rebuild. Never delete
- [ ] 2.7 Unit-test D2 directly: append a row that joins an existing dedup group and assert it is not usage-bearing; append a row with an older timestamp under a newer id (the Time Machine backfill shape) and assert the group's totals are unchanged

## 3. Background refresher

- [ ] 3.1 Spawn an interval task at hub start that calls `mirror.refresh()`, and kick off the initial build when no mirror exists
- [ ] 3.2 On refresh failure, leave the mirror intact, log distinguishably from a credential failure, and continue. The task MUST NOT exit the process and MUST NOT feed the `28P01` strike counter that governs process exit (design D6)
- [ ] 3.3 Test that repeated refresh failures leave the process running and statistics still served from the existing mirror

## 4. Port the rollups

- [ ] 4.1 Port `materialize_scope` to the mirror, computing the derived booleans at refresh rather than per request (design D3)
- [ ] 4.2 Port the eight rollups — totals, active session time, daily, heatmap, models, providers, top projects, and the single-pass tools/skills/subagents — keeping the existing function signatures and `history-core` return types
- [ ] 4.3 Preserve every semantic the current implementation documents: `cost_usd` reported as "where reported" rather than coalesced to zero; active session time as summed gaps with the 30-minute idle cap, not raw span; `conversational` gating on `role IS NOT NULL`; and outcome resolution as `COALESCE(r.is_error, u.is_error, false)` with outcomes collapsed by `bool_or` on `(session_id, tool_use_id)` **before** the join, or the join fans out
- [ ] 4.4 Delete the Postgres rollup implementation. It is replaced, not kept as a fallback (design D4)
- [ ] 4.5 Port the existing endpoint tests onto the new path with the same fixtures and expectations — no second implementation retained for comparison

## 5. Endpoint surface

- [ ] 5.1 Serve `/v1/stats/*` from the mirror; return `503` with `Retry-After` while warming
- [ ] 5.2 Add `X-Stats-Mirror-Refreshed-At` and `X-Stats-Mirror-Age-Seconds` to every `/v1/stats/*` response. Headers, not body fields — a new field on the `history-core` stat types breaks struct literals in `src-tauri`, which `rust-tests.yml` builds (design D5)
- [ ] 5.3 Add unauthenticated `GET /v1/healthz/stats` reporting readiness and age, non-success past the threshold, in the Gatus-consumable shape `/v1/healthz/journal` already uses
- [ ] 5.4 **Gitea #26**: change `/v1/stats/sessions/{id}` to `Path<String>` resolved through `resolve_session_ref`, so a UUID and a row id both work and an unknown UUID returns `404` rather than a parse `400`
- [ ] 5.5 Test the warming path (`503` then `200` without a restart), the staleness headers, the health endpoint at both states, and #26's four cases: known row id, known UUID, unknown row id, unknown UUID

## 6. Verification gate

- [ ] 6.1 Re-point the `hub-analytics` oracle harness (task 6.1 of that change) at the DuckDB implementation
- [ ] 6.2 Require the same verdict as before: exact agreement on token, cost, message, session and activity fields, with both documented divergences still one-directional — hub success rate ≤ oracle's (D10), hub tool counts ≤ oracle's (D12)
- [ ] 6.3 Compare per-project statistics for a multi-machine, multi-path identity and per-session statistics for a tool-heavy session, as the original gate did
- [ ] 6.4 Record the comparison in this change. **This gate must pass before #23 is started** — Deliverable 2 deletes the oracle, and after that there is nothing independent to diff against
- [ ] 6.5 Measure the deployed endpoint and record it against the 18.0 s baseline: archive-wide, the webapp's default 30-day window, per-project, and per-session

## 7. Quality gate and deploy

- [ ] 7.1 Frontend gate: `pnpm tsc --build .`, `pnpm vitest run`, `pnpm lint`, `pnpm run i18n:validate`
- [ ] 7.2 Rust gate: archive crates with `TEST_DATABASE_URL` set (`cargo test -p history-core -p archive-protocol -p hub -p sync-daemon -- --test-threads=1`), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, and the desktop crate from `src-tauri/`
- [ ] 7.3 Regenerate sqlx offline metadata for the new Postgres reads (`cargo sqlx prepare`) — `SQLX_OFFLINE=true` is what CI builds with
- [ ] 7.4 Merge to `main` and cut a `cchv-v*` release per CLAUDE.md § Release Process, remembering `Cargo.lock` (the recurring miss `just sync-version` does not cover)
- [ ] 7.5 Relay the deploy to infra as a §2b binary swap, **split per home-network #34** so no single handler run exceeds the 900 s ceiling. The relay MUST state that `/v1/stats/*` returns `503 warming` for roughly four minutes after the swap while the first mirror builds, and that this is expected rather than a rollback trigger
- [ ] 7.6 Relay a Gatus check for `/v1/healthz/stats` to infra
- [ ] 7.7 Verify live: statistics load in the deployed webapp, the measured timings from 6.5 hold in production, and the staleness headers advance as the refresher runs
- [ ] 7.8 Close Gitea #24 and #26 with the measured before/after
