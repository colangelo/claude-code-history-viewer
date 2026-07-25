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

## Open Questions

- None blocking. Batch size and the `messages (message_id)` index shape are to be
  chosen against `EXPLAIN` on the real table during tasks 3.2 and 3.4.
- Whether per-session statistics need pagination. The desktop had
  `PaginatedTokenStats` for listing many sessions' stats; the hub endpoints here
  are single-session and single-project, so pagination is deferred until the UI
  shows it is needed.
