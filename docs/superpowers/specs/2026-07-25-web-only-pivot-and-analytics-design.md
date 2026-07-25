# Web-only pivot: cut the desktop, move analytics to the hub, add retrospective synthesis

**Date:** 2026-07-25
**Status:** approved design, not yet planned
**Scope:** three sequential deliverables (analytics → cut → synthesis)

## Why

The fork ships the web viewer + archive stack. The Tauri desktop distribution
is already retired (CLAUDE.md § Desktop app), the updater is dormant, and the
per-platform WebUI server binaries are dispatch-only with no consumer. What
remains is dead weight that every quality gate and CI run still pays for.

The goal for the project, in the owner's words, is *"a way to archive, analyze
and build with time the history of my projects and my operations with agents."*
Deleting the desktop serves that goal only if analytics survive the cut — so
analytics move to the hub rather than dying with `src-tauri`.

## Measured starting state

Numbers taken from the tree at `1e3143fa`:

| Area | Size | Fate |
|---|---|---|
| `src-tauri/` | 25.1k LOC Rust, 79 `#[tauri::command]`, 40 `webui-server` cfg gates | delete |
| — `commands/stats.rs` | 6.0k LOC, **zero** `history_core::` calls | delete (semantics port to SQL) |
| — `commands/archive.rs` | 2.2k LOC — *local* `~/.claude-history-viewer/archives/`, unrelated to the hub archive | delete |
| — `wsl.rs` | 197 LOC, referenced only by `src-tauri` | delete |
| Desktop-only frontend | 203 files / 38.9k LOC unreachable from `archive-main.tsx` | delete ~181; **~22 analytics files (4.4k LOC) migrate** — see D5 |
| Already-dead frontend | 19 files / 2.3k LOC reachable from neither entrypoint | delete |
| `@tauri-apps/*` npm deps | 8 packages | remove |
| Archive webapp graph | 289 files / 48.2k LOC (283 shared with desktop today) | keep |
| `crates/history-core` | 36.4k LOC | keep — upstream supply chain |
| `crates/hub` / `sync-daemon` / `protocol` | 7.5k / 2.4k / 0.4k LOC | keep |

Dependency direction is already one-way: `src-tauri → history-core`. No crate
under `crates/` references `src-tauri`, so removing it cannot break the archive
stack.

## Decisions

### D1 — Full cut to the archive stack

Delete `src-tauri` and the desktop-only frontend. `index.html`/`main.tsx` are
removed; `archive.html`/`archive-main.tsx` become the sole entrypoint and
`vite.archive.config.ts` folds back into `vite.config.ts`. One build, one
artifact.

### D2 — `--serve` does not survive

The WebUI server embeds `#[folder = "../dist"]` — the desktop frontend itself.
Serving the archive webapp locally is pointless, since that bundle's only
network dependency is the hub. `--serve` is therefore deleted, not re-homed.
`cchv-find` §3 loses "headless server"; §1 (hub API) and §2 (raw files) are
unaffected.

### D3 — `crates/cli` keeps headless export

`history-core` already owns `export.rs` (38K) and `cli_args.rs`; only the
dispatch in `src-tauri/src/lib.rs::run` and `cli.rs` needs re-homing. New thin
binary crate:

```
cchv export <session-id|/abs/path.jsonl> [--format html|json] [--output FILE]
```

`--session` (a GUI preload hint) dies with the GUI. This keeps `cchv-find` §3
working in reduced form, and must land **before** the deletion so nothing
regresses.

### D4 — Analytics move to the hub, not to cliproxyapi

Usage Keeper (cliproxyapi, deployed 2026-07-07) and the cchv hub are **not
substitutes**:

| | Usage Keeper | cchv hub |
|---|---|---|
| Source | proxied HTTP requests | transcripts on disk, all machines |
| Coverage from | 2026-07-07 | 2025-10-09 (Time Machine backfill) |
| Providers | only what is pointed at the proxy keys | Claude Code, Codex, OpenCode, Copilot CLI/Desktop, VS Code Copilot, Pi |
| Attribution | API key → model → tokens → cost | project identity, session, machine, provider, model, tool, sidechain |
| Answers | spend | work |

`ANTHROPIC_BASE_URL` and `OPENAI_BASE_URL` are unset on this machine and absent
from `~/.claude/settings.json`, so Keeper does not observe the primary Claude
Code work at all. Even fully routed, a proxied request carries no project or
session identity, so it structurally cannot answer "the history of my projects".

The same limit applies to Langfuse / Helicone / OpenLLMetry — gateway- or
SDK-instrumented, forward-only, not project-attributed. Claude Code's own OTel
(`CLAUDE_CODE_ENABLE_TELEMETRY`, currently `0`) would give session-level metrics
but is Claude-Code-only, forward-only, duplicates data already stored at full
fidelity, and reverses a deliberate telemetry-off posture.

**Keeper stays as the spend view. The hub gains the work view.**

### D5 — Analytics are SQL, not a port

`messages` already carries every column the metrics need:

```
model, input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens,
cost_usd, duration_ms, stop_reason, provider, machine_id, timestamp,
is_sidechain, content JSONB, raw JSONB
```

Almost all of `stats.rs` is mmap + SIMD line-scanning + parallel file-walking
whose only job is extracting those columns from JSONL — work the ingest path has
already done. That machinery is deleted outright.

Three things carry over:

1. **The metric types are already safe.** `SessionTokenStats`, `DailyStats`,
   `ToolUsageStats`, `ActivityHeatmap`, `ProjectStatsSummary`, `ModelStats`,
   `ProviderUsageStats`, `GlobalStatsSummary` live in
   `crates/history-core/src/models/stats.rs`, which survives the cut. The
   response contracts carry over free, so the migrated analytics UI keeps its
   types unchanged and only swaps its data source.
2. **The token dedup rule** (`stats.rs:1500`) — the one piece of non-obvious
   correctness:
   ```
   key = session_id | "m:" + message_id      (fallback: session_id | "u:" + uuid)
   ```
   One assistant response recurs across multiple JSONL lines carrying the *same*
   `usage` block. Counting once per `message_id` is what prevents
   double-counting. **A naive `SUM(input_tokens)` over `messages` over-reports.**
3. **Tool and skill extraction** — tool name, success rate, and the Claude
   `Skill` tool keyed by `input.skill`.

### D6 — Sequencing: analytics before deletion

Land the hub analytics first and verify its numbers against the live desktop
analytics on the same data. The old implementation is the oracle; once the SQL
rollups are proven to agree, `src-tauri` can be deleted with confidence.

### D7 — Upstream: parser supply chain only

Port `upstream:src-tauri/src/commands/*.rs` → `crates/history-core/src/providers/*.rs`
by hand, as already happens. Upstream has no `crates/`, and the fork is 311
ahead / 51 behind, so no wholesale merge has been possible for some time. No
further PRs back; #445 (merged 2026-07-11) was the last. **Triage the 51-commit
backlog for parser fixes before the cut**, while the old layout is still present
to diff against.

### D8 — Capabilities that die without replacement

Accepted knowingly:

- **Local reader** — reading `~/.claude` with no hub. Raw-file access
  (`cchv-find` §2) remains.
- **WSL / Windows provider discovery** — `wsl.rs` is `src-tauri`-only. The
  project becomes macOS-client + Linux-server shaped.
- **`--serve`** — per D2.

## Deliverable 1 — Quantitative analytics on the hub

**Migration `0005`:**

- Promote `raw->'message'->>'id'` to a real `message_id` column with an index.
  It exists today only inside `raw` JSONB. (`hub/src/search.rs`'s `message_id`
  is the DB row id — unrelated. `message_key` is a SHA-256 *row* content hash
  for ingest dedup — a different concern that will not collapse duplicate usage
  blocks.) Without this column the dedup rule of D5.2 cannot be expressed.
- `message_tool_uses(message_ref BIGINT REFERENCES messages(id), tool_name,
  skill_name, is_error)` populated at ingest, so tool statistics do not scan
  JSONB. (`message_ref` is the DB row id; the new `message_id` column above is
  the Anthropic `msg_…` id. Two distinct things — do not conflate them.)

**Endpoints**, returning the existing `history-core` stat types:

```
GET /v1/stats/global
GET /v1/stats/projects/{identity_key}
GET /v1/stats/sessions/{id}
```

All rollups apply `DISTINCT ON (session_id, message_id)` before summing usage.

**Metrics** (all four requested):

- **Cost & tokens over time** — per day/week/month, split by model, provider,
  machine.
- **Per-project work history** — sessions, messages, tokens, duration, active
  periods per project identity, folded across moved/cloned repos via the
  existing git-fingerprint identity work.
- **Agent operations** — tool-use frequency, success/error rate, Claude Skill
  invocations by name.
- **Activity rhythm** — hour/day-of-week heatmap, session durations, streaks
  and gaps.

**Frontend:** an Analytics tab alongside Journal | Browse.

The existing analytics UI is ~4.4k LOC across ~22 files
(`AnalyticsDashboard/`, `hooks/analytics/`, `store/slices/analyticsSlice.ts`,
`services/analyticsApi.ts`, `utils/sessionAnalytics.ts`) and — importantly —
`package.json` carries **no chart library**. `ActivityHeatmap.tsx`,
`DailyTrendChart.tsx` and siblings are hand-rolled, so they port cleanly with no
new dependency.

This UI **migrates** into the webapp tree during this deliverable rather than
being deleted in Deliverable 2, which reduces that deletion from 203 files to
roughly 181. What changes is the data source (Tauri `invoke` → hub HTTP) and the
store wiring; the client-side aggregation helpers in
`AnalyticsDashboard/utils/*` are largely superseded by SQL and should be dropped
as they are replaced, not ported wholesale.

**Verification gate:** hub output must agree with the desktop analytics on the
same data before Deliverable 2 begins.

## Deliverable 2 — The cut

Executed only after Deliverable 1's verification gate passes.

1. Land `crates/cli` and verify `cchv export` (D3).
2. Delete `src-tauri/`, the ~181 remaining desktop-only frontend files (203
   minus the ~22 migrated by Deliverable 1), and the 19 already-dead files.
3. Remove the 8 `@tauri-apps/*` deps, `tauri.conf.json`, all `tauri-plugin-*`
   crates, and the `src-tauri` workspace member.
4. Collapse entrypoints and vite configs (D1).
5. Prune i18n: `settings` (501 keys) and `update` (65 keys) across 5 locales.
   `renderers` (255 keys) stays — the archive graph shares 283 files with the
   desktop, including all message renderers. **`analytics` (132 keys) also
   stays**, since Deliverable 1 migrates the analytics UI into the webapp.
6. Retire the `tauri-axum-parity-checker` agent; its bug class (issues #340,
   #355) no longer exists with one command surface.
7. CI: `server-release.yml` drops the dispatch-only WebUI binary matrix, so a
   `cchv-v*` tag publishes exactly the hub binary + `cchv-webapp.tar.gz`.
   `update-flow-tests.yml` is deleted. `frontend-tests.yml`, `rust-tests.yml`,
   `archive-tests.yml` lose their tauri paths.
8. Docs: CLAUDE.md loses the retired-desktop caveats; `docs/archive/deployment.md`
   unchanged (the hub deploy path is untouched).

The live m4m deployment is unaffected until a normal release — the webapp is not
modified by this deliverable.

## Deliverable 3 — Retrospective synthesis

Answers "what is occupying my mind and my time, over time — project
trajectories, turns, pivots, and focus". Qualitative, so it builds on
`journal_entries` (daily, per project, LLM-distilled: `headline`, `summary`,
`topics[]`, `open_questions[]`, `session_ids`, `model`) and `journal_embeddings`
(bge-small, live since v0.12.0) rather than on `messages` alone.

**Pattern:** a second-order distiller — the same shape as the nightly
`dev.cchv.distiller`, one level up.

**Four computed signals, produced before any generation:**

- **Topic mass over time** — `topics[]` frequency per project per window,
  weighted by time and tokens actually spent (requires Deliverable 1).
- **Trajectory drift** — centroid distance between consecutive windows'
  `journal_embeddings`. A pivot is a measured event: a window whose semantic
  centroid diverges sharply from its predecessor.
- **Open-question lifecycle** — which entries in `open_questions[]` recur, which
  resolve, which silently vanish. Dropped threads and abandoned directions, with
  receipts.
- **Focus concentration** — distinct active projects/topics per window (entropy
  or HHI) and context-switch rate.

**Then** an LLM narrates over that evidence plus the window's journal text.
Computed signals are stored alongside the narrative so any claim is auditable.

**Schema — `journal_periods`:**

```
period_kind   TEXT CHECK (period_kind IN ('week','month','quarter','adhoc'))
period_start  DATE
period_end    DATE
scope         TEXT              -- project identity_key, or 'global'
headline      TEXT
narrative     TEXT
pivots        JSONB             -- measured drift events + explanation
threads       JSONB             -- resolved / dropped / carried-forward
signals       JSONB             -- the computed evidence, for audit
model         TEXT
generated_at  TIMESTAMPTZ
generated_snapshot PG_SNAPSHOT  -- same dirty-detection stance as journal_entries
search_text / text_search
```

Partial unique index on `(period_kind, period_start, scope)` where
`period_kind <> 'adhoc'`, so scheduled periods are singular while ad-hoc windows
may overlap freely.

**Cascade:** weekly synthesizes from daily entries, monthly from weekly,
quarterly from monthly. Cheaper and more coherent than re-reading all dailies at
every level.

**Cadences:** weekly, monthly, and quarterly on schedule; plus on-demand over an
arbitrary window (`period_kind = 'adhoc'`, cached by window hash).

**API:** `/v1/journal/periods/pending` + POST, mirroring today's dirty-group
protocol; a synthesize-now endpoint for ad-hoc windows.

**Frontend:** a Retrospect view alongside Journal | Browse | Analytics.

**Backfill:** history reaches 2025-10-09 — roughly 41 weeks, 10 months, and 3
quarters. That count is **per scope**, and with ~15 project identities plus
global the naive upper bound is ~860 generated entries. Two things bound it in
practice, and both must be explicit rather than assumed:

- Periods are generated only where underlying daily entries exist, and most
  projects are inactive in most weeks. The real count is far below the upper
  bound but is not known until measured.
- A minimum-density floor (skip a window with fewer than N daily entries) and a
  scope policy (global-only first, per-project for active identities) are the
  two knobs. **Measure the real distribution before running any backfill**, and
  run it as an explicit one-time batch — never folded into the first scheduled
  run.

## Risks

| Risk | Mitigation |
|---|---|
| SQL rollups silently disagree with the old analytics | D6 verification gate against the live desktop implementation before deletion |
| `message_id` backfill is expensive on the existing corpus | Measure first; a generated column avoids a rewrite if the JSONB path is stable |
| Upstream parser fixes stranded in the 51-commit backlog | Triage before the cut (D7), while the old layout is still diffable |
| Deleting ~181 files makes future upstream merges impossible | Already true in practice — 311 ahead / 51 behind, and every sync is already a hand-port |
| Synthesis narrative drifts from evidence | Computed signals stored in `signals JSONB` alongside the narrative; claims are checkable |
| Backfill token cost | Explicit one-time batch, not part of the scheduled run |

## Out of scope

- Renaming or re-identifying the repository. Unblocked by all of the above and
  deferred with no deadline; when done, the MIT notice and
  `authors = ["JaeHyeok Lee"]` lineage carry over.
- Replacing Usage Keeper. It stays as the spend view.
- Reinstating desktop, WSL, or `--serve` support.

## Implementation artifacts

Each deliverable gets its own OpenSpec change under `openspec/`, consistent with
the repo's existing capability specs (`archive-ingestion`, `journal-entries`,
`project-identity`, …). This document is the design record behind all three.
