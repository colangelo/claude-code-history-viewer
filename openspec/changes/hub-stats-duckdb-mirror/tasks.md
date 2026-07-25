# hub-stats-duckdb-mirror — tasks

This file is the implementation plan. Measurements referenced below are in
`design.md`; do not re-derive them.

## 1. Dependency and groundwork

- [x] 1.1 **Verify timezone support before anything else is built.** **DONE — no redesign needed; the premise was outdated.** Probed `duckdb-rs` 1.10505.0 (DuckDB 1.5.5) with `bundled`, in-memory, offline: `LOAD icu` **fails** (not bundled, would need a runtime download), but IANA tz support is in **core** and everything the rollups need works — `Europe/Rome` gives `+01` in January and `+02` in July (real DST, not a fixed offset), `Asia/Kolkata` gives the correct half-hour offset, and `strftime` / `extract(hour|dow)` / `SET TimeZone` all behave. Recorded as design D7. **Constraint that follows: the port MUST NOT emit `LOAD icu`** — the spike SQL did, and it would fail in the bundled crate
- [x] 1.2 Add `duckdb` (bundled) to `crates/hub`. **Measured on an isolated probe crate first, to keep a failed answer out of the workspace: +40 MB** (hub asset is 14 MB at v0.15.0 → ~54 MB), **statically linked with no `libduckdb` dylib reference** (so the codesign-aware §2b swap is unaffected), **~873 s CPU / 1m38s wall** on this machine. A 3-core macos-14 runner implies roughly +5 min, taking the release job from its recent 4m15s–5m38s to ~10 min. Acceptable — the prebuilt-libduckdb fallback is not needed. *Remaining sub-task: actually add the dependency to `crates/hub` when group 2 starts*
- [x] 1.3 Confirm disk headroom on m4m for the mirror. **DONE: 121 GiB available** (1.8 TiB volume); `~/.config/cchv` is currently 713 MB of which `staging/` is 535 MB. A 119 MB mirror is immaterial — note the figure in the deploy relay anyway
- [x] 1.4 Add mirror settings to `HubConfig`: file path (default under the hub config dir), refresh interval, overlap window size, DuckDB `memory_limit` **and `threads`** (m4m is shared with the distiller and daemon), and the staleness threshold used by the health endpoint. All optional with defaults, so an existing `hub.toml` keeps working untouched

## 2. The mirror module

- [x] 2.1 Create `crates/hub/src/mirror.rs` owning the file and nothing else: `open_or_create`, `state() -> Warming | Ready { max_id, refreshed_at }`, `refresh() -> RefreshReport`. It MUST NOT know anything about statistics. *`rebuild()` is deliberately NOT here yet — it lands with 2.8, which owns the build-aside/atomic-swap behaviour*
- [x] 2.2 Define the mirror schema: the 13 `messages` columns the rollups read plus the derived `usage_row` and `conversational` booleans; `sessions` **including the provider session identifier**; `projects` **plus the project-identity tables** — every identifier a stats endpoint accepts (row id, session UUID, identity key) must resolve from the mirror alone, or the Postgres-outage scenario silently holds for only one endpoint; `message_tool_uses`; `message_tool_results`. Primary keys on mirrored row ids so inserts can be idempotent
- [ ] 2.3 Implement the full build: pull each table from Postgres and populate the mirror. Measured at 227 s for 2.8M messages, network-bound. *Partly covered already: `refresh()` on an empty mirror walks the whole table in `FETCH_CHUNK` batches, so this task is now about the explicit cold-build path and its progress reporting, not the row movement*
- [x] 2.4 Implement incremental refresh with the **overlap re-scan**: fetch rows from an overlap window behind the watermark (by id, or by ingest `created_at` over the last several minutes — ids are allocated at insert but commit out of order under concurrent ingest, so `WHERE id > max_id` alone skips rows forever), insert with `INSERT OR IGNORE`, and compute the derived booleans for genuinely new rows only (design D2)
- [ ] 2.5 Guard refresh with single-flight — a tick arriving while one runs is skipped, not queued
- [x] 2.6 On an unopenable or corrupt mirror, move the file aside under a timestamped name and rebuild. Never delete
- [x] 2.7 **DONE — and verified non-vacuous:** re-running with `overlap_rows = 0` (i.e. the naive `WHERE id > max_id`) fails on exactly the late-commit assertion, so the test genuinely covers the bug rather than passing by construction. Unit-test D2 in all three shapes: a row that joins an existing dedup group is not usage-bearing; a row with an older timestamp under a newer id (the Time Machine backfill shape) leaves its group's totals unchanged; **a lower-id row becoming visible after higher ids were already mirrored is picked up by the overlap re-scan exactly once**
- [ ] 2.8 Add the `hub mirror rebuild` subcommand (main.rs already has the subcommand pattern): build a fresh mirror aside, swap atomically, keep serving from the old file throughout. Test that a `message_id` UPDATE on existing Postgres rows is reflected after rebuild with no over-counted usage. Document the rebuild requirement in `docs/archive/deployment.md` **next to the `backfill-analytics` instructions** — the two operations travel together (design D2)

## 3. Background refresher

- [ ] 3.1 Spawn an interval task at hub start that calls `mirror.refresh()`, and kick off the initial build when no mirror exists
- [ ] 3.2 On refresh failure, leave the mirror intact, log distinguishably from a credential failure, and continue. The task MUST NOT exit the process and MUST NOT feed the `28P01` strike counter that governs process exit (design D6)
- [ ] 3.3 Test that repeated refresh failures leave the process running and statistics still served from the existing mirror

## 4. Port the rollups

- [ ] 4.1 Port `materialize_scope` to the mirror, computing the derived booleans at refresh rather than per request (design D3)
- [ ] 4.2 Port the eight rollups — totals, active session time, daily, heatmap, models, providers, top projects, and the single-pass tools/skills/subagents — keeping the existing function signatures and `history-core` return types
- [ ] 4.3 Preserve every semantic the current implementation documents: `cost_usd` reported as "where reported" rather than coalesced to zero; active session time as summed gaps with the 30-minute idle cap, not raw span; `conversational` gating on `role IS NOT NULL`; and outcome resolution as `COALESCE(r.is_error, u.is_error, false)` with outcomes collapsed by `bool_or` on `(session_id, tool_use_id)` **before** the join, or the join fans out
- [ ] 4.4 Run rollups on `spawn_blocking` with cloned connections (or a small pool) — duckdb-rs is synchronous, and without this every stats request parks a tokio worker for ~0.4 s. Nothing in the gates would catch it; it has to be built in, not discovered under load
- [ ] 4.5 **Differential gate (design D4): run BEFORE deleting anything.** Both implementations over the same live data, outputs diffed field by field. Expect exact equality everywhere except windowed comparisons of the ≤10 boundary groups D3 measures. Record the result in this change
- [ ] 4.6 Delete the Postgres rollup implementation. It is replaced, not kept as a fallback (design D4)
- [ ] 4.7 Port the existing endpoint tests onto the new path with the same fixtures and expectations — no second implementation retained

## 5. Endpoint surface

- [ ] 5.1 Serve `/v1/stats/*` from the mirror; return `503` with `Retry-After` while warming
- [ ] 5.2 Add `X-Stats-Mirror-Refreshed-At` and `X-Stats-Mirror-Age-Seconds` to every `/v1/stats/*` response. Headers, not body fields — a new field on the `history-core` stat types breaks struct literals in `src-tauri`, which `rust-tests.yml` builds (design D5)
- [ ] 5.3 Add unauthenticated `GET /v1/healthz/stats` reporting readiness, age, **and watermark lag (mirror max id vs Postgres `max(id)` — a cheap PK lookup)**, non-success past the staleness threshold, in the Gatus-consumable shape `/v1/healthz/journal` already uses. Age alone cannot distinguish a healthy mirror from one silently skipping rows
- [ ] 5.4 **Gitea #26**: accept both a session UUID and a row id on `/v1/stats/sessions/{id}` (`Path<String>`), resolved **against the mirror** — same acceptance rule as `resolve_session_ref`, but without touching Postgres at serve time, so an unknown UUID returns `404` rather than a parse `400` and the Postgres-outage scenario stays true for all three endpoints
- [ ] 5.5 Test the warming path (`503` then `200` without a restart), the staleness headers, the health endpoint in all three states (warming, fresh, lagging), and #26's four cases: known row id, known UUID, unknown row id, unknown UUID

## 6. Verification gate

- [ ] 6.1 Re-point the `hub-analytics` oracle harness (task 6.1 of that change) at the DuckDB implementation
- [ ] 6.2 Require the same verdict as before: exact agreement on token, cost, message, session and activity fields, with both documented divergences still one-directional — hub success rate ≤ oracle's (D10), hub tool counts ≤ oracle's (D12). **One new allowance:** windowed comparisons may differ for the ≤10 dedup groups that straddle a date boundary (design D3) — that delta is accepted and recorded, not a failure
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
- [ ] 7.7 Verify live: statistics load in the deployed webapp, the measured timings from 6.5 hold in production, the staleness headers advance as the refresher runs, and the watermark lag on `/v1/healthz/stats` tracks near zero under live ingest
- [ ] 7.8 Close Gitea #24 and #26 with the measured before/after
