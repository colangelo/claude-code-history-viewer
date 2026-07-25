## Context

Deliverable 1 of the web-only pivot. The strategic reasoning — why the desktop
is being cut, why cliproxyapi's Usage Keeper is not a substitute, why analytics
must precede deletion — lives in
`docs/superpowers/specs/2026-07-25-web-only-pivot-and-analytics-design.md` and is
not repeated here. This document covers only *how* to build the hub-side
analytics.

Current state: `messages` already carries `model`, the four token columns,
`cost_usd`, `duration_ms`, `stop_reason`, `provider`, `machine_id`, `timestamp`,
`is_sidechain`, plus normalized `content` and raw-fidelity `raw` JSONB. The
retired desktop implementation (`src-tauri/src/commands/stats.rs`, 6.0k LOC)
computes the same metrics by mmap-scanning JSONL; it stays in the tree until
Deliverable 2 and serves as the verification oracle.

Two constraints shape everything below:

1. **The corpus already exists.** ~9 months of history is ingested (back to
   2025-10-09 via Time Machine backfill). Any new derived data must be
   *backfillable over stored rows*, because re-ingesting the whole archive to
   populate a column is not acceptable.
2. **Usage must be deduplicated.** One assistant response occupies several
   stored rows carrying an identical `usage` block. A plain `SUM` over-reports.

## Goals / Non-Goals

**Goals:**

- Serve token/cost, per-project, tool/skill, and activity statistics from
  Postgres, matching the desktop oracle's numbers.
- Make the provider message id a first-class queryable column, since dedup
  cannot be expressed without it.
- Extract tool/skill invocations once, at ingest, so query time never scans JSONB.
- Migrate the existing analytics UI into the webapp with its types intact.

**Non-Goals:**

- Deleting `src-tauri` (Deliverable 2) or retrospective synthesis (Deliverable 3).
- Changing `crates/history-core/src/models/stats.rs`. The stat types are reused
  verbatim; that is what keeps the UI port mechanical.
- Replacing Usage Keeper, which measures spend through the proxy and has no
  project attribution.
- Byte-exact `raw` passthrough (a pre-existing planned enhancement — see risks).

## Decisions

### D1 — The provider message id is read from `raw->>'messageId'`

**This corrects the arc design doc, which said `raw->'message'->>'id'`.** That
path returns NULL for every row.

`raw` is not the original JSONL line. `crates/sync-daemon/src/convert.rs:94`
sets `raw: serde_json::to_value(m)` where `m` is a **normalized, flat**
`ClaudeMessage` — the nested `message` object of the source format is gone. In
`ClaudeMessage`, the field is `message_id` with `#[serde(rename = "messageId")]`
(`crates/history-core/src/models/message.rs:132`), so the JSONB key is
`messageId` at the top level.

Its runtime semantics are what matters, and the source comment ("File history
snapshot fields") is misleading. `TryFrom<RawLogEntry>` (message.rs:203, 240)
computes `message_id: msg.id.clone().or(log_entry.message_id)` — the **assistant
`msg_…` id wins**, falling back to the file-history-snapshot `messageId`. This
dual-purpose field is exactly what `stats.rs:1500` dedups on, so reproducing the
oracle means reproducing this precedence.

*Alternatives considered:*

- **Add `message_id` to `IngestMessage`** (the wire type currently has no such
  field). Rejected *for this deliverable*: it needs a daemon rollout on every
  machine, and it does nothing for the 9 months already stored, which still need
  a `raw`-derived backfill. Worth doing later as a robustness follow-up, at which
  point the column becomes authoritative and `raw` extraction is the fallback.
- **`raw->'message'->>'id'`** — wrong, as established above.

### D2 — Do not "clean up" the dual-purpose field

A future refactor separating the assistant id from the snapshot `messageId`
would silently change dedup grouping. File-history-snapshot rows carry no
`usage`, so they cannot corrupt token totals today, but the coupling must be
documented at both ends rather than tidied away.

### D3 — `message_id` is a real backfilled column, not a generated column

A `GENERATED ALWAYS AS (raw->>'messageId') STORED` column is tempting — it
self-maintains and cannot drift. It is rejected because it welds the schema to
the current `raw` format, which the ingestion spec already flags as slated to
change to byte-exact passthrough. Under that change a generated column silently
starts producing NULLs (or the wrong value) archive-wide with no migration step
to notice it.

A plain nullable `TEXT` column populated at ingest and backfilled once keeps the
extraction rule in Rust, where the `raw`-format change will have to be handled
anyway.

### D4 — Tool extraction is hub-side at ingest, from normalized `content`

The daemon has the parsed message and could ship a compact tool list, which
would be cheaper. Rejected for the same reason as D1: daemon-side extraction
cannot populate the existing corpus without a full re-ingest, and it makes tool
statistics depend on every machine's daemon version.

Extracting hub-side from the stored normalized `content` means one code path
serves both live ingest and the backfill of existing rows, and old daemons need
no upgrade. The wire protocol is unchanged by this deliverable.

Query-time extraction (a GIN index over `raw`) was rejected outright: tool stats
are requested per project and globally over the whole archive, so this is the
hot path, not an occasional lookup.

### D5 — Dedup is a CTE applied before aggregation, not a post-filter

Every usage rollup runs over a deduplicated relation:

```sql
WITH deduped AS (
  SELECT DISTINCT ON (session_id, COALESCE(message_id, uuid, id::text))
         session_id, input_tokens, output_tokens,
         cache_creation_tokens, cache_read_tokens, cost_usd, ...
  FROM messages
  WHERE <scope and window predicates>
  ORDER BY session_id, COALESCE(message_id, uuid, id::text), seq
)
SELECT ... FROM deduped
```

The `COALESCE` chain reproduces the oracle's precedence exactly: `message_id`,
else `uuid`, else count the row unconditionally (the row `id` is unique, so a
row with neither identifier is never collapsed into another).

Counting distinct identifiers in the aggregate instead (`SUM(...) FILTER` with a
window function) was considered; the CTE is chosen because it keeps the dedup
rule in one place that every endpoint composes, rather than repeated per metric
where one omission silently over-reports.

### D6 — Statistics are scoped by project identity, not project path

Per-project endpoints key on `identity_key` and fold every path and machine
belonging to it, reusing the existing `project-identity` grouping. Keying on
`project_path` would report a moved or cloned repository as several unrelated
projects — the exact defect the identity work already fixed for browsing.

### D7 — Timezone is a request parameter

Daily buckets and the hour/day heatmap are meaningless in UTC for a user in
`Europe/Rome`. Bucketing is done server-side with an IANA timezone supplied by
the caller (defaulting to UTC), so Postgres does the `AT TIME ZONE` conversion
next to the index rather than shipping rows to the client to re-bucket.

### D8 — Verification against the oracle is a first-class task, not a spot check

The gate for Deliverable 2 is that hub and desktop agree. This is done by
running both over the same scope and window and diffing the stat structs — the
same types on both sides make this a structural comparison, not eyeballing
charts. Disagreement blocks the cut.

One documented exception: tool success rate, per D10. Everything else must match.

### D9 — The UI migrates with its types, and loses its math

`crates/history-core/src/models/stats.rs` is unchanged, so the TypeScript types
mirroring it (`src/types/analytics.ts`) are unchanged too, and the hand-rolled
charts (no chart library in `package.json`) port as-is. What changes:
`services/analyticsApi.ts` switches from Tauri `invoke` to hub HTTP, and
`AnalyticsDashboard/utils/*` — client-side aggregation over locally-read
messages — is deleted as each metric moves to SQL, not ported.

### D10 — Tool success rate is computed correctly, and deliberately diverges from the oracle

The oracle does not actually measure this. `stats.rs:556-566` reads `is_error`
off the `tool_use` **content item**, where the key never exists — `is_error`
belongs to the `tool_result` item that arrives in a *later user message*. With
`unwrap_or(false)`, every content-array invocation scores as a success, so the
desktop's reported success rate is ~100% by construction. Only the secondary
path (top-level `toolUse` + `toolUseResult.is_error`, both on the same record,
stats.rs:576-587) correlates correctly.

Invocations therefore store their `tool_use_id`, outcomes are extracted into
`message_tool_results (tool_use_id, is_error)`, and the rollup resolves them
with a LEFT JOIN, preferring the joined outcome and falling back to the
same-record `is_error`:

```sql
COALESCE(r.is_error, u.is_error, false)
```

Both rows always land in the same session, so no cross-batch reconciliation is
needed — whichever message is ingested second finds the other already stored.

**This makes success rate the one metric expected NOT to match the oracle.** The
verification gate (D8) carves it out explicitly: every other field must match
exactly, and a mismatch anywhere else is still a bug. Replicating the oracle
faithfully was rejected because a metric that reports 100% regardless of what
happened is not worth migrating.

### D11 — The invocation↔outcome join MUST be scoped by session

`tool_use_id` is **not unique across the archive**. Joining
`message_tool_results` to `message_tool_uses` on `tool_use_id` alone fans out
across every session and machine that ever reused the same id string, silently
multiplying invocations and corrupting success rate.

Found by `analytics_extract_test`, which produced three rows where one was
expected because sibling tests in the shared database had written results under
the same `toolu_1` id. That is a test-database artifact in miniature, but the
same collision is inevitable in a real archive spanning many machines and
providers.

Both tables therefore denormalize `session_id` from their owning message, and
the join key is `(session_id, tool_use_id)` with a matching index on each side.
An invocation and the result reporting on it always share a session, so this is
exactly as precise as the pairing itself. Joining through `messages` twice would
have avoided the denormalization, but the join is on the whole-archive hot path
and the composite index is what keeps it selective.

### D12 — The top-level `toolUse` shape is a fallback, not an addition

On Claude records the top-level `toolUse` is a **redundant restatement** of the
content-array `tool_use` on the same record. Counting both doubles every tool
count.

Measured on pg1 (2% sample of 2.64M messages, 2026-07-25): of 2,551 assistant
rows carrying both shapes, the top-level name matched an array `tool_use` name
in **2,551 — 100%, zero divergences**, and every one held exactly one array item.
Top-level `toolUse` appears on assistant rows only; `tool_result` items appear on
user rows.

The extractor therefore emits the top-level row only when the record produced no
content-array invocation, preserving coverage for records that carry only that
shape. The oracle runs both paths unconditionally and double-counts; per D10's
reasoning that is not worth reproducing.

**Gate consequence:** tool/skill/subagent *counts* join success rate as
deliberately divergent — hub counts will be roughly half the oracle's wherever
Claude records carry both shapes. Token, cost, message, session and activity
figures must still match exactly.

> Probe note for whoever re-measures: `raw ? 'toolUse'` is **useless as a
> filter** — it is true for every row, because `ClaudeMessage::tool_use` has no
> `skip_serializing_if`, so `raw` always carries `"toolUse": null`. Filter on
> `raw->'toolUse'->>'name' IS NOT NULL` instead.

## Risks / Trade-offs

- **`uuid` fallback is not stable across re-parses** → `history-core` fills a
  missing provider uuid with a random v4 (`message.rs:217`, and
  `convert.rs` excludes `uuid` from `message_key` for exactly this reason). Within
  a stored corpus the value is fixed, so queries are self-consistent; but a
  re-ingest after a re-parse can regroup dedup for providers lacking real uuids.
  Accepted: identical behavior to the oracle. Providers with real ids are
  unaffected.
- **Backfilling `message_id` over the whole corpus may be expensive** → measure
  the row count and the JSONB extraction cost first, run it in batches, and add
  the index concurrently. If it proves prohibitive, the column can be populated
  forward-only with backfill deferred, at the cost of incomplete history until
  it completes.
- **`raw` is slated to become byte-exact original lines** → that change breaks
  the `raw->>'messageId'` extraction. Mitigated by D3 (rule lives in Rust, not
  in a generated column) and by cross-referencing this decision from the
  ingestion spec so the enhancement cannot land without addressing it.
- **`cost_usd` is absent for most providers** → cost totals must be presented as
  "cost where reported", never silently treated as zero, or per-provider cost
  comparisons will be misleading.
- **Tool-extraction backfill and live ingest can disagree** → they share one code
  path (D4); a divergence is a bug, and the idempotence scenario in the ingestion
  spec is the regression test.
- **Two agents on this repo** → this change is developed in the
  `feature/hub-analytics` worktree to avoid the shared-tree contention that
  already dropped a commit once.
- **Amending migration `0005` invalidates existing test databases** → sqlx
  rejects a changed migration with `VersionMismatch(5)`. That is correct
  behavior and the remedy is to recreate the scratch/test database. It is only
  available because `0005` is unreleased and applied nowhere else; **once it
  ships to pg1, any change must be a new migration.**

### Unrelated pre-existing issue, found while running the suite

`crates/hub/tests/embed_sweep_test.rs` is not isolated against itself:
`deleting_embedding_rows_self_heals` and
`regenerated_entry_re_embeds_on_hash_change` assert global `SweepStats.embedded`
counts, but `journal_entries` keys on `(entry_date, project_path)` with no
machine scoping, so the file's five tests contaminate each other when run
concurrently. They fail under a bare `cargo test -p hub` and pass under
`--test-threads=1`.

This is masked — not fixed — by CI and the release gate both mandating
`--test-threads=1`. Nothing in this change touches journal entries or
embeddings, and the contamination is within that one binary rather than across
binaries, so it is neither caused nor newly exposed here. Recorded rather than
silently dropped; worth its own issue, and out of scope for this change.

## Migration Plan

1. Migration `0005`: add nullable `messages.message_id`; create
   `message_tool_uses`. Both additive, no rewrite of existing columns.
2. Deploy hub with extraction active at ingest — new rows populate immediately,
   before any backfill runs.
3. Backfill in batches: `message_id` from `raw->>'messageId'`, tool rows from
   stored `content`. Progress is observable and resumable; the job is
   idempotent and re-runnable.
4. Create the `message_id` index concurrently once the backfill has drained.
5. Enable `/v1/stats/*`; verify against the desktop oracle (D8).
6. Ship the webapp Analytics tab as a `static_dir` swap.

**Rollback:** the endpoints are additive and the webapp tab degrades on `404`,
so reverting the hub binary is sufficient; the migration can stay in place
harmlessly since nothing else reads the new column or table.

## Measured corpus (pg1, read-only, 2026-07-25)

Task 3.1. Extrapolations are from a 2% `TABLESAMPLE SYSTEM`.

| Measure | Value |
|---|---|
| `messages` rows | **2,643,609** (6.4 GB total relation size) |
| `sessions` / `projects` | 4,012 / 193 |
| Rows carrying `messageId` | **10.6%** → ~280,000 to backfill |
| Rows where `raw->'message'->>'id'` hits | **0** — confirms D1 against production |
| Content-array `tool_use` items | ~4.8% of rows → ~128,000 invocations |
| `tool_result` items | ~4.8% of rows → ~127,000 outcomes |
| Provider mix | claude 2,632,835 · codex 9,122 · pi 1,506 · zed 124 · cursor 116 · gemini 7 |

`messageId` presence (10.6%) tracks the assistant-row share (10.9%) closely,
which is the expected shape: assistant responses carry the id, other record types
do not.

Backfill is therefore ~280k targeted UPDATEs plus ~255k derived-row inserts over
a 6.4 GB table — batched and resumable, not a single statement. Sizing the
batches and the index build is task 3.2/3.4.

## Backfill executed and verified on pg1 (2026-07-25)

Task 3.5. Migration `0005` applied 09:51:22Z, backfill completed 10:04:24Z, with
live ingest running throughout and the hub reporting `db: up` the whole time.

```
scanned 2,655,854 · message_ids 271,752 · tool_uses 132,342 · tool_results 132,168
```

| Check | Predicted | Actual |
|---|---|---|
| `message_id` fill rate | 10.6% (2% sample) | **10.22%** (271,752 / 2,659,881) |
| Invocations | ~128k | **132,342** |
| Outcomes | ~127k | **132,168** |
| D1 dual-purpose split | assistant ids dominate | 268,380 `msg_…` + 3,372 snapshot ids |

**Transcript diff (the actual 3.5 check).** Sessions were diffed against their
`~/.claude/projects/**.jsonl` on m4m. In every settled session **`only_disk` = 0**
— every provider message id present on disk is in the archive with the right
value. That is the only direction that can indict extraction, and it is clean.

The reverse direction is non-zero and is *correct*: the archive holds ids the
current file no longer does. Two causes, both confirmed:

- a **live** session showed a clean prefix boundary — DB held assistant messages
  at file positions 1–143, disk had 1–202, the remainder simply pending the next
  ingest cycle;
- a **settled** session held 142 extra ids whose timestamps fall *inside* the
  file's own window, i.e. the on-disk transcript has been thinned by context
  compaction while the archive retained the originals.

An archive that is a strict superset of the live file is the entire point of the
project, so this asymmetry is a feature under test, not a defect.

**D10 payoff — success rate is now informative** rather than the oracle's flat
~100%: Bash 94.2% over 68,865 uses (4,008 real errors), Glob 91.7%, Read 97.9%,
TaskUpdate 99.9%.

**D12 holds on real data:** zero rows where a top-level restatement
(`tool_use_id IS NULL`) coexists with a content-array invocation on the same
message. The 116 messages carrying repeated tool names are genuine multi-call
messages — predominantly lowercase `read`/`bash` from Codex/Pi records, all with
non-NULL `tool_use_id`.

**Rollback, if ever needed** (nothing pre-existing was modified):

```sql
UPDATE messages SET message_id = NULL;   -- or ALTER TABLE ... DROP COLUMN
TRUNCATE message_tool_uses, message_tool_results;
```

## The dedup rule, measured on the live archive (2026-07-25)

Read-only on pg1, the whole archive:

| | Naive `SUM` | Deduplicated |
|---|---|---|
| Tokens | 40,962,494,393 | **19,864,402,131** |
| Rows | 2,670,049 | 2,526,385 |

**A naive rollup over-reports tokens by 51.5%.** Only 5.4% of rows collapse, but
they are the assistant responses carrying repeated `usage` blocks, so they
dominate the totals. This is the number that justifies migration `0005` and the
`message_id` column: without them the dedup key cannot be expressed, and every
token and cost figure the archive reports is roughly double the truth.

## A Non-Goal amended: cost fields added to the stat types

The design listed "changing `crates/history-core/src/models/stats.rs`" as a
Non-Goal, because reusing the types verbatim is what keeps the UI port
mechanical. But **none of them carried cost**, and cost-over-time is one of the
four requested metrics, so there was no home for it.

Resolved by *additive* fields only — `total_cost_usd` / `cost_usd` (both
`Option<f64>` with `skip_serializing_if`, so absent rather than null) plus
`cost_reported_messages` on the two summaries. Existing JSON snapshots are
byte-identical and the migrated frontend types still compile unchanged; only
consumers that want cost need to know about them.

`Option`, never `0.0`: most providers report no cost, and a zero would read as
"free" rather than "unknown". `cost_reported_messages` exists so a consumer can
say "cost across N of M messages" instead of implying full coverage.

`total_reasoning_tokens` stays 0 — `TokenUsage` in history-core has no reasoning
field at all, so the oracle's value is 0 too. Parity, not a gap.

## Endpoint performance, measured against pg1 (2026-07-25)

A hub was run read-only against the live archive (no `embed_model_dir`, so the
embedding sweeper never started; only `/v1/stats/*` was called).

| Request | Messages in scope | Latency |
|---|---|---|
| `/v1/stats/global`, first implementation | 2,532,204 | **30.9 s** |
| `/v1/stats/global`, after materializing once | 2,534,911 | **13.9 s** |
| `?from=2026-05-01` | 2,355,921 | 12.7 s |
| `?from=2026-07-20` | 614,498 | 5.1 s |

**Why it was 30.9 s.** Each rollup re-derived the dedup CTE, and a dedup pass is
a parallel seq scan plus an external merge sort spilling ~60 MB per worker
(`EXPLAIN` on pg1). A summary runs ~10 rollups, so the archive was sorted ten
times per request. Materializing the deduped set once into an `ON COMMIT DROP`
temp table and aggregating over that halved it, with byte-identical output.

**It is still not interactive**, and windowing helps less than expected because
the archive's mass is recent: 2.25M of 2.53M messages fall in the last month, so
"last 30 days" is most of the corpus. Cost is ~linear at ~8 µs/message.

**Per-statement breakdown (pg1, global scope, 2.54M messages):**

| Statement | Time |
|---|---|
| Materialization (dedup + sort) | 3.56 s |
| daily | 2.20 s |
| totals (+ range + duration folded) | 1.30 s |
| providers | 1.27 s |
| top projects | 1.24 s |
| heatmap | 0.97 s |
| tools/skills/subagents (one pass) | 0.70 s |
| models | 0.26 s |

Two guesses were measured and refuted before this table existed, which is why it
is here: the three tool queries were folded into one `VALUES`-lateral pass on
the assumption that the repeated join dominated (it is 0.70 s — it never did),
and that change moved the endpoint 13.9 s → 13.7 s. **The real pattern is
`count(DISTINCT …)`**, which dominates `daily`, `providers` and `top_projects`.

Consequently the expression index is worth ~15%, not the fix: it targets only
the 3.56 s materialization and would not eliminate it. The folding is kept
anyway — one query for three collections is simpler than three near-identical
ones — but it is honestly recorded as a performance no-op.

Remaining options, in increasing order of commitment:

1. **Expression index** `(session_id, COALESCE(message_id, uuid, id::text), id)`
   so `DISTINCT ON` becomes an ordered index scan and the external sort
   disappears. A new migration (`0005` is deployed; amending it is no longer
   available). Removes the sort, but the ~10 aggregate scans over 2.5M rows
   remain — expect seconds, not milliseconds.
2. **Fold independent aggregates into fewer statements** so the temp table is
   scanned ~4 times instead of ~10.
3. **Precomputed rollup tables** refreshed on a schedule, with the live query
   reserved for narrow windows. This is what actually reaches sub-second, and it
   is a genuine design change rather than a tuning pass.

Not decided here — it is a product judgement about whether an analytics page may
take several seconds on first load. **Tracked with full research and sizing in
cchv Gitea #24**, including two further measurements taken while writing it up:

- `work_mem` on pg1 is **4 MB**, so the dedup sort spilling ~60 MB per worker is
  unavoidable at that setting. `SET LOCAL work_mem` inside the stats transaction
  is the cheapest experiment — no schema change, no production write — and it
  likely makes the expression index redundant.
- A precomputed hourly rollup would be **10,559 rows** and a tool-daily rollup
  **4,488** — ~15k rows versus 2.54M messages, a ~240× reduction, which is what
  makes option 3 the only path to sub-second.

## An honest gap: no cost data exists

`cost_reported_messages` is **0** across the entire archive: not one message
carries `costUSD`. The cost plumbing (D-amendment above) is correct and tested,
but it currently has nothing to report, because no provider in this archive
emits per-message cost. Cost-over-time will stay empty until some provider
does — which is an argument for keeping cliproxyapi's Usage Keeper as the spend
view, exactly as the arc design concluded.

## Verification gate result (2026-07-25) — PASSES

Task group 6. The desktop oracle was built with `--features webui-server` and run
headlessly (`--serve --host 127.0.0.1 --no-auth`) beside a read-only hub, and the
two were compared per session.

**Comparison basis.** The oracle reads *local files*; the hub reads the *archive*,
which is deliberately a superset (compaction, Time Machine backfill, three
machines). Comparing arbitrary sessions would fail on differences that are not
bugs. So the gate runs only on sessions where the DB and the on-disk transcript
hold an **identical set of provider message ids** — 2 of 40 sampled m4m sessions
qualified, itself a measure of how much the archive preserves beyond the live
files.

**Result — every token field and every tool count matches exactly:**

| Field | Session 275 | Session 276 |
|---|---|---|
| `total_input_tokens` | 18,865 = 18,865 | 19,676 = 19,676 |
| `total_output_tokens` | 32,839 = 32,839 | 30,552 = 30,552 |
| `total_cache_creation_tokens` | 142,857 = 142,857 | 88,049 = 88,049 |
| `total_cache_read_tokens` | 5,189,936 = 5,189,936 | 1,138,833 = 1,138,833 |
| `total_tokens` | 5,384,497 = 5,384,497 | 1,277,110 = 1,277,110 |
| tool counts (per tool) | identical, 83 total | identical, 22 total |

### Two real bugs the gate caught

Both would have shipped silently, and neither was visible to the unit tests.

1. **Tool counts were ~4× low.** The rollup joined tools to the *deduplicated*
   set. But a single assistant response is streamed across several records
   sharing one `message.id` and one `usage` block, and **each record carries
   different content blocks** — so its tool calls are distinct events. Dedup is
   correct for usage and wrong for tools. The materialized scope now keeps every
   scoped row and flags `usage_row`; usage rollups filter on it, tool joins do
   not.
2. **The outcome join fanned out.** Task 4.4b said to guard against an invocation
   with more than one recorded outcome; the guard was never written. A duplicate
   `tool_result` record inflated Bash by exactly 1 in a session with 83 real
   invocations. Outcomes are now collapsed with `bool_or(is_error)` grouped by
   `(session_id, tool_use_id)` before joining.

### Documented divergences (3)

- **`ToolUsageStats.success_rate`** (D10) — oracle reports a flat 1.0; the hub
  reports real rates (e.g. 0.951, 0.625). One-directional: hub ≤ oracle.
- **Tool counts vs the oracle's double-count** (D12) — not observed in these two
  sessions, whose records carry the top-level restatement consistently; the
  guard remains.
- **`message_count`** — oracle 262/94, hub 161/44. Two compounding causes, both
  correct: the archive is cumulative (934 stored rows vs 569 lines currently on
  disk for session 275), and the two count different things. The oracle counts
  raw parsed records; the hub counts *logical conversational messages* —
  deduplicated, and excluding bookkeeping record types (`mode`,
  `permission-mode`, `attachment`, `custom-title`, …) which were 680 of 934 rows
  in session 275. Filtering on `role IS NOT NULL` is provider-agnostic, unlike
  matching `type` values that differ per provider.

**Deliverable 2 is unblocked**: the numbers that matter reproduce the oracle
exactly, and every divergence is understood and deliberate.

### Side effect worth noting

Adding cost fields to the history-core stat types broke **four struct
initializers** in `src-tauri/src/commands/stats.rs`. "Additive" held for serde
but not for struct literals, and `rust-tests.yml` builds that crate — so CI on
this branch would have failed. Fixed (the desktop predates cost, so `None` is
correct); the crate builds again.

## Open Questions

- None blocking. Batch size and the `messages (message_id)` index shape are to be
  chosen against `EXPLAIN` on the real table during tasks 3.2 and 3.4.
- Whether per-session statistics need pagination. The desktop had
  `PaginatedTokenStats` for listing many sessions' stats; the hub endpoints here
  are single-session and single-project, so pagination is deferred until the UI
  shows it is needed.
