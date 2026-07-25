## 1. Schema

- [x] 1.1 Write `migrations/0005_analytics.sql`: add nullable `messages.message_id TEXT`, and create `message_tool_uses` (surrogate id, `message_ref BIGINT NOT NULL REFERENCES messages(id) ON DELETE CASCADE`, `tool_name TEXT NOT NULL`, `skill_name TEXT`, `is_error BOOLEAN NOT NULL DEFAULT false`, ordering column, `created_at`)
- [x] 1.2 Add a `UNIQUE (message_ref, seq)` constraint on `message_tool_uses` so re-extraction is idempotent rather than accumulating rows
- [x] 1.3 Add the `message_tool_uses (tool_name)` and `(skill_name)` indexes needed for the usage rollups; leave the `messages (message_id)` index to task 3.4 (created concurrently after backfill)
- [x] 1.4 Verify the migration applies cleanly on a scratch database and is additive only (no rewrite of existing `messages` columns)

## 2. Ingest extraction

- [x] 2.1 Add a `message_id` extraction helper in `crates/hub` reading `raw->>'messageId'` (design D1 — NOT `raw->'message'->>'id'`), with a unit test covering: assistant `msg_…` id present, snapshot-only `messageId`, and neither
- [x] 2.2 Add a tool-invocation extractor over the normalized `content`, returning `(tool_name, tool_use_id, skill_name, subagent_type, is_error)` per invocation; `skill_name` from `input.skill` when the tool is `Skill`, `subagent_type` from `input.subagent_type` when the tool is `Agent`, and `is_error` only for the top-level `toolUse` shape whose result rides the same record
- [x] 2.3 Add a tool-outcome extractor for `tool_result` content items, returning `(tool_use_id, is_error)` per result (design D10 — the invocation does not carry its own outcome)
- [x] 2.4 Wire all three into the ingest path so new rows populate `message_id`, `message_tool_uses`, and `message_tool_results` on insert
- [x] 2.5 Make tool-row and result-row writes idempotent with the message upsert (re-ingesting a message must not accumulate rows) — covers both re-ingest scenarios in the ingestion spec
- [x] 2.6 Unit-test extraction against fixtures for: messages with no tool use, multiple invocations in one message, `Skill` invocations naming a skill, `Agent` invocations naming a subagent type, same-record `toolUseResult` errors, and `tool_result` items referencing an invocation
- [x] 2.7 Test that a result ingested in a batch not containing its invocation is stored and resolves once the invocation arrives (order independence)

## 3. Backfill

- [x] 3.1 Measure first: `COUNT(*)` over `messages`, count of rows where `raw->>'messageId'` is non-null, and `EXPLAIN` the extraction — record the numbers in the change before sizing the job (design Open Questions)
- [x] 3.2 Write a resumable, idempotent batch backfill for `message_id` over existing rows, re-runnable without double work
- [x] 3.3 Extend the backfill to populate `message_tool_uses` and `message_tool_results` from stored `content`, sharing the extractors from 2.2 and 2.3 (design D4 — one code path for live and backfill)
- [x] 3.4 ~~Create the index concurrently once the backfill has drained~~ — **superseded**: sqlx applies all pending migrations at startup, so a later migration cannot be ordered after the backfill. A PARTIAL index in `0005` instead, which starts empty (column all-NULL at creation) so the build indexes nothing; the backfill maintains ~280k entries incrementally. Recorded in the migration
- [x] 3.5 Verify post-backfill on pg1: spot-check a session whose assistant messages are known, confirming `message_id` matches the `msg_…` ids in its transcript. **DONE 2026-07-25** — migration + backfill executed on pg1 under live traffic; transcript diff shows `only_disk`=0 on every settled session. Results in design.md. A second sweep still runs at 8.4 to catch messages ingested by the old hub binary in the meantime

## 4. Dedup and rollups

- [ ] 4.1 Implement the dedup CTE as a single reusable query fragment (design D5): `DISTINCT ON (session_id, COALESCE(message_id, uuid, id::text))`, ordered so one row per identifier survives
- [ ] 4.2 Test the dedup rule directly: repeated `usage` blocks sharing a `message_id` count once; distinct `uuid`s with no `message_id` each count once; rows with neither are never collapsed
- [ ] 4.3 Implement token/cost rollups (input, output, cache-creation, cache-read, reasoning, total, cost) composed over the dedup CTE, with `cost_usd` reported as "where reported" rather than coalesced to zero (design risk)
- [ ] 4.4 Implement tool, skill, and subagent usage rollups over `message_tool_uses`, producing `ToolUsageStats` for `most_used_tools`, `most_used_skills` (by `skill_name`) and `most_used_subagents` (by `subagent_type`)
- [ ] 4.4b Resolve success rate with a LEFT JOIN to `message_tool_results` on `tool_use_id`, as `COALESCE(r.is_error, u.is_error, false)` (design D10); guard the join against fan-out if an invocation id ever has more than one recorded outcome
- [ ] 4.5 Implement daily buckets (`DailyStats`) and the hour/day heatmap (`ActivityHeatmap`) with server-side `AT TIME ZONE` conversion from a caller-supplied IANA timezone defaulting to UTC (design D7)
- [ ] 4.6 Implement per-model (`ModelStats`) and per-provider (`ProviderUsageStats`) breakdowns

## 5. Endpoints

- [ ] 5.1 Add a `stats` module to `crates/hub` and mount `/v1/stats/global`, `/v1/stats/projects/{identity_key}`, `/v1/stats/sessions/{id}` behind the existing read-token auth
- [ ] 5.2 Support optional inclusive `from`/`to` date-window params and the `tz` param on all three endpoints
- [ ] 5.3 Fold per-project statistics across every path and machine of an identity, reusing the existing `project-identity` grouping (design D6)
- [ ] 5.4 Return `404` for unknown identity keys and unknown session ids; `401` for missing or invalid tokens
- [ ] 5.5 Serialize responses as the existing `history-core` stat types with no changes to `crates/history-core/src/models/stats.rs`
- [ ] 5.6 Add endpoint tests covering auth rejection, date-window narrowing, identity folding across machines, and both not-found cases

## 6. Verification gate

- [ ] 6.1 Build a comparison harness that runs the desktop analytics and the hub endpoints over the same scope and window and diffs the stat structs field by field
- [ ] 6.2 Compare global statistics; investigate and resolve every discrepancy — a difference is a bug in the new implementation until proven otherwise. **Carve-outs (two, both deliberate):** `ToolUsageStats.success_rate` differs because the oracle scores every content-array invocation as a success (D10); tool/skill/subagent *counts* differ because the oracle counts the top-level `toolUse` and the content-array `tool_use` as two invocations when they are one (D12). Assert both divergences are one-directional — hub success rate ≤ oracle's, hub tool counts ≤ oracle's — and that token, cost, message, session and activity fields all match exactly
- [ ] 6.3 Compare per-project statistics for at least one multi-machine, multi-path identity, and per-session statistics for a tool-heavy session
- [ ] 6.4 Record the comparison results in the change; **Deliverable 2 is blocked until this passes**

## 7. Webapp Analytics tab

- [ ] 7.1 Move `src/types/analytics.ts`, `src/components/AnalyticsDashboard/`, `src/hooks/analytics/`, `src/store/slices/analyticsSlice.ts`, `src/services/analyticsApi.ts`, `src/utils/sessionAnalytics.ts` into the graph reachable from `archive-main.tsx`
- [ ] 7.2 Rewrite `analyticsApi.ts` to call the hub over HTTP with the stored hub config and read token, replacing Tauri `invoke`
- [ ] 7.3 Delete `AnalyticsDashboard/utils/*` client-side aggregation as each metric is served by SQL — do not port it (design D9)
- [ ] 7.4 Add the Analytics view to the webapp navigation alongside Journal and Browse, with scope (whole archive / single identity) and date-window controls
- [ ] 7.5 Handle a `404` from the statistics endpoints with an explanatory "hub needs upgrading" message that leaves Journal and Browse fully usable
- [ ] 7.6 Confirm the `analytics` i18n namespace resolves in all five locales for the migrated view; add any new keys across `en`, `ko`, `ja`, `zh-CN`, `zh-TW` and run `pnpm run i18n:validate`
- [ ] 7.7 Verify the archive webapp still builds standalone (`just archive-web-build`) with no Tauri imports pulled into the bundle

## 8. Quality gate and deploy

- [ ] 8.1 Run the frontend gate: `pnpm tsc --build .`, `pnpm vitest run`, `pnpm lint`, `pnpm run i18n:validate`
- [ ] 8.2 Run the Rust gate: `cargo test -- --test-threads=1`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`
- [ ] 8.3 Merge `feature/hub-analytics` into `main` and cut a `cchv-v*` release per CLAUDE.md § Release Process
- [ ] 8.4 Relay the deploy to infra: migration against pg1, hub binary swap on m4m per `docs/archive/deployment.md` §2b, then the webapp `static_dir` swap
- [ ] 8.5 Verify live: statistics load in the deployed webapp and agree with the pre-deploy comparison
