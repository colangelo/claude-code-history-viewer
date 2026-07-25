# Proposal: hub-analytics

## Why

The fork ships the web viewer + archive stack; the Tauri desktop is retired and
is being deleted (Deliverable 2 of the web-only pivot — design record:
`docs/superpowers/specs/2026-07-25-web-only-pivot-and-analytics-design.md`).
Analytics is the only capability that would die in that cut without a
replacement path: `src-tauri/src/commands/stats.rs` is 6.0k LOC of per-machine,
local-file analytics with **no hub equivalent** — the hub exposes projects,
sessions, messages, search, journal and identities, but nothing about tokens,
cost, tools or activity.

Rebuilding it on the hub is not a port. The `messages` table **already ingests
every column the metrics need** (`model`, the four token columns, `cost_usd`,
`duration_ms`, `stop_reason`, `provider`, `machine_id`, `timestamp`,
`is_sidechain`), because almost all of `stats.rs` is mmap + SIMD line-scanning
whose only job is extracting exactly those fields from JSONL — work the ingest
path has already done. What is left is SQL. The result is strictly more capable
than what it replaces: cross-machine instead of per-machine, and covering the
full archive back to 2025-10-09 (Time Machine backfill) instead of whatever
files happen to sit on the local disk.

This also unblocks the deletion. Deliverable 2 is deliberately gated on this
change, so the existing desktop implementation can serve as a verification
oracle before it is removed.

## What Changes

- **Migration `0005`** promotes the provider message id (`raw->'message'->>'id'`,
  the Anthropic `msg_…` id) out of JSONB into a real indexed
  `messages.message_id` column. This is **required for correctness**, not
  convenience: the token dedup rule cannot be expressed without it.
- **A `message_tool_uses` table**, populated at ingest, so tool and skill
  statistics do not scan `raw` JSONB at query time.
- **New hub endpoints** `GET /v1/stats/global`,
  `GET /v1/stats/projects/{identity_key}`, `GET /v1/stats/sessions/{id}`,
  returning the stat types that already exist in
  `crates/history-core/src/models/stats.rs` (`GlobalStatsSummary`,
  `ProjectStatsSummary`, `SessionTokenStats`, `DailyStats`, `ToolUsageStats`,
  `ModelStats`, `ProviderUsageStats`, `ActivityHeatmap`). Response contracts are
  unchanged, so the existing UI types port untouched.
- **The token dedup rule carries over as a spec'd requirement.** One assistant
  response recurs across multiple JSONL lines carrying the *same* `usage` block;
  `stats.rs:1500` counts it once per `session_id | message_id` (falling back to
  `uuid`). A naive `SUM(input_tokens)` over `messages` **over-reports**. Every
  usage rollup MUST deduplicate before summing.
- **The analytics UI migrates** (~4.4k LOC, ~22 files) from the desktop tree into
  the archive webapp as an Analytics tab alongside Journal | Browse.
  `package.json` carries **no chart library** — `ActivityHeatmap.tsx`,
  `DailyTrendChart.tsx` and siblings are hand-rolled, so they port with no new
  dependency. What changes is the data source (Tauri `invoke` → hub HTTP) and the
  store wiring; the client-side aggregation helpers in
  `AnalyticsDashboard/utils/*` are superseded by SQL and are dropped as they are
  replaced.
- **A verification gate**: hub output must agree with the live desktop analytics
  on the same data before Deliverable 2 deletes the desktop.

Nothing here is breaking: all endpoints are additive, migration `0005` is
additive, and the webapp change is a new tab. Older hubs simply lack
`/v1/stats/*`, which the webapp must degrade against rather than crash.

## Capabilities

### New Capabilities

- `archive-analytics`: hub-side aggregate statistics over the archive — token
  and cost rollups, per-project work history, tool and skill usage, and activity
  rhythm — served from Postgres with usage deduplicated per provider message id.

### Modified Capabilities

- `archive-ingestion`: the schema gains an indexed `messages.message_id` column
  carrying the provider message id, and ingest additionally persists extracted
  tool/skill invocations to a `message_tool_uses` table. Both are spec-level
  storage guarantees that analytics depends on, not implementation details.
- `static-archive-webapp`: the webapp gains an Analytics view alongside Journal
  and Browse, including its behavior when the connected hub predates
  `/v1/stats/*`.

## Impact

**Schema** — `migrations/0005_*.sql`: additive column on `messages` plus a new
`message_tool_uses` table. Backfilling `message_id` over the existing corpus
needs measuring before it runs; a generated column avoids a table rewrite if the
JSONB path proves stable.

**Rust** — `crates/hub`: new `stats` module and routes; the ingest path extracts
tool/skill rows and the provider message id.
`crates/history-core/src/models/stats.rs` is reused as-is and is **not** modified.

**Frontend** — `src/components/AnalyticsDashboard/`, `src/hooks/analytics/`,
`src/store/slices/analyticsSlice.ts`, `src/services/analyticsApi.ts`,
`src/utils/sessionAnalytics.ts`, `src/types/analytics.ts` move into the webapp
graph reachable from `archive-main.tsx`. The `analytics` i18n namespace (132
keys × 5 locales) is retained rather than pruned in Deliverable 2.

**Deployment** — a hub binary swap on m4m per `docs/archive/deployment.md` §2b,
plus a webapp `static_dir` swap. The migration runs against pg1.

**Out of scope** — deleting `src-tauri` (Deliverable 2), retrospective synthesis
(Deliverable 3), and replacing cliproxyapi's Usage Keeper, which observes
proxied requests with no project attribution and remains the separate spend view.
