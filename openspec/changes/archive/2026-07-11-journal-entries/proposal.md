# Proposal: journal-entries

## Why

The archive holds raw transcripts back to 2025-10-09, but recall over them is
grep-shaped: `/v1/search` ranks individual messages, so answering "what did we
do in project X around date D, and what threads did we drop?" means
reconstructing days from message-level hits. A distillation stage — per-(date,
project) journal entries generated from archived sessions — gives the archive
high-quality retrieval units (distilled entries beat raw transcripts for
recall) and captures `open_questions` (dropped threads) as a queryable signal.
Cherry-picked from [engineering-notebook](https://github.com/prime-radiant-inc/engineering-notebook)
(Apache-2.0): we port its entry schema and summarization idea (`src/summarize.ts`),
not the tool (it re-implements our ingest/sync/store, worse). Tracked as
issue #12 (relay 2026-07-11 from home-network).

## What Changes

- New Postgres table `journal_entries` (migration `0002_journal_entries.sql`):
  per-(date, project_path) entries folded **across machines**, with headline,
  summary, topics, open_questions, session provenance, a `generated_at`
  dirty-detection watermark, a `skip` status for non-substantive days, and
  their own tsvector + GIN index.
- New hub endpoints: `GET /v1/journal/pending` (catch-up work list, read-auth),
  `POST /v1/journal/entries` (upsert, machine-token auth like ingest),
  `GET /v1/journal/entries` (browse, read-auth).
- `/v1/search` gains an **additive** `journal` result block and a `scope`
  parameter (`all` default | `messages` | `journal`). `scope=messages`
  preserves today's exact response shape — non-breaking for the archive
  webapp and the `cchv-find` skill.
- New distiller job: `scripts/cchv-distill.py` (PEP 723/uv standalone script)
  run by launchd `dev.cchv.distiller` on m4m — fetches pending groups, runs
  `claude -p` (Haiku, single turn) per group, POSTs entries back. Catch-up
  based, not tick-based: work list derived from data, so sleep and
  late-arriving syncs only delay entries, never drop them. Forward from ship
  date automatically; `--backfill [--from DATE] [--limit N]` processes history
  newest-first in bounded, resumable chunks.
- Out of scope (deliberately): dropped-threads report over `open_questions`
  (data captured from day 1, report is a later change); archive webapp journal
  UI; `cchv-find` skill update (follow-up commit in the CONTEXT repo).

## Capabilities

### New Capabilities

- `journal-entries`: per-(date, project) journal entries distilled from
  archived sessions — schema, generation contract (fold rules, skip sentinel,
  catch-up/dirty semantics), storage, and the journal read/write API.

### Modified Capabilities

- `archive-search-api`: `/v1/search` gains the `scope` parameter and the
  additive `journal` result block; backward compatibility of the existing
  message-hit shape becomes an explicit requirement.

## Impact

- **Code**: `crates/hub` (new `journal.rs`, router additions in `lib.rs`,
  `search.rs` scope param), `migrations/0002_journal_entries.sql`,
  `scripts/cchv-distill.py` (new), launchd plist + deployment docs
  (`docs/archive/deployment.md`).
- **API**: additive only; `scope=messages` regression-locks the current
  search shape.
- **Consumers**: archive webapp and `cchv-find` unaffected until they opt in;
  `cchv-find.eval.toml` (CONTEXT repo) can measure the recall delta after
  backfill chunks.
- **Ops**: new launchd job on m4m (launchd-resilience-conformant, bao-first
  hub token from `kv/infra/cchv/hub-tokens`); Haiku calls consume Max-plan
  quota — backfill is deliberate and bounded, never automatic.
- **Version**: ships as `cchv-v0.6.0` (minor).
