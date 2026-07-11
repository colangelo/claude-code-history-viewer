# Design: journal-entries

## Context

The archive stack (hub on m4m + per-Mac sync-daemons + pg1 Postgres) holds
raw transcripts back to 2025-10-09, searchable per-message via
`websearch_to_tsquery` over `messages.search_text`. Issue #12 (relay
2026-07-11) ports the genuinely novel piece of engineering-notebook
(Apache-2.0): per-(date, project) journal entries as distilled retrieval
units. Design decisions below were made interactively 2026-07-11; the specs
in `specs/` are the contract — this doc records how and why.

House constraints: launchd jobs on m4m conform to the launchd-resilience
contract (never prompt headless, bao-first secrets, degrade-don't-loop —
see `scripts/cchv-launch.sh` and macos-setup `docs/launchd-resilience.md`);
standalone scripts are PEP 723/uv; the hub is the only component with DB
credentials.

## Goals / Non-Goals

**Goals:**

- Distilled, searchable journal entries for every substantive (date, project)
  group, folded across machines, generated catch-up-style so downtime and
  late syncs only delay entries.
- Additive API: nothing existing breaks; `scope=messages` locks the current
  search shape with a regression test.
- Bounded, deliberate backfill of the 9-month archive.
- Entries contract-complete from day 1 (`open_questions` captured) so later
  consumers (dropped-threads report, webapp UI, cchv-find) need no schema
  change.

**Non-Goals:**

- Dropped-threads report, webapp journal UI, cchv-find skill update (all
  follow-ups; the last lives in the CONTEXT repo).
- Semantic/pgvector search over entries (home-network#15 territory).
- Distilling on machines other than m4m, or inside the hub/daemon binaries.

## Decisions

1. **Distiller = external script + `claude -p`, not hub-native LLM calls.**
   Follows the proven relay-poller/evals-runner pattern; keeps the always-on
   hub free of LLM latency, retries, and key management; uses Max-plan quota
   instead of API spend. Alternatives rejected: hub-native Rust API calls
   (couples service uptime to LLM behavior, burns credits); daemon-side
   distillation (N machines × LLM config, and cross-machine folds become
   impossible).

2. **Fold key = `(entry_date, project_path)` across machines.** `~/_sync`
   paths are identical on every Mac, and recall is "what happened in project
   X on date D", not "…on machine M". Machine provenance stays reachable via
   the entry's session ids. Alternative rejected: per-machine entries mirror
   the schema's scoping but split one day's work into near-duplicates that
   pollute search.

3. **Catch-up, not tick-based.** The work list is a SQL view of the data
   (missing entry, or session ingested after `generated_at`), served by
   `GET /v1/journal/pending`. The launchd schedule is merely "when to drain".
   This one mechanism covers distiller downtime, m4m sleep, and machines
   syncing days of backlog. `skip` rows are watermarks so dead days aren't
   re-judged every run.

4. **Session→day assignment by first message + `day_start_hour` (04:00).**
   Whole sessions are assigned to one logical day (no message-level
   splitting): simpler provenance (`session_ids[]` stays exact), and
   multi-day sessions are rare enough that first-message assignment is the
   right 90% answer. Ported from engineering-notebook's logical-day fold.

5. **Additive `journal` block in `/v1/search`, `scope` param.** A unified
   ranked list (kind discriminator) is cleaner retrieval semantics but breaks
   every existing consumer's hit parsing; a separate search endpoint makes
   the recall win opt-in per caller and easy to forget. The additive block is
   non-breaking (consumers ignore unknown fields) while still landing in the
   one endpoint every caller already hits. `scope=messages` gives a cheap
   compat escape hatch and a regression-test anchor.

6. **Write path via `POST /v1/journal/entries` with machine-token auth.**
   Reuses the ingest auth model (the distiller is just another push client
   with a `hub-tokens` credential) rather than inventing a new role. Hub-side
   validation (status shape, 3–8 topics, session ids exist) keeps garbage out
   even if the script misbehaves.

7. **Backfill is explicit, newest-first, bounded.** `--backfill --from --limit`
   reuses the same pending query with a wider horizon. Newest-first because
   recent history is queried most; bounded chunks so quota burn is controlled
   and a prompt bug found mid-sweep doesn't cost the whole archive. The
   automatic path only ever looks forward from ship date.

8. **Entry generation prompt/schema ported from engineering-notebook
   `src/summarize.ts`** (headline / 2–5 sentence summary / 3–8 topics /
   open_questions / SKIP sentinel), adapted to `claude -p --output-format
   json` single-turn with schema validation in the script before POST.

## Risks / Trade-offs

- [LLM output variability breaks schema] → validate in the distiller, reject
  and leave pending, log; malformed output never reaches the hub. Hub
  validates independently (defense in depth).
- [Quota burn from backfill] → backfill never automatic, bounded by
  `--limit`, newest-first; eval (`cchv-find.eval.toml`) measures whether the
  recall win justifies continuing.
- [Dirty-detection misses] → pending query compares `generated_at` against
  session ingest time (`sessions.updated_at`), not message timestamps —
  late-arriving *old* messages still bump `updated_at` at ingest. Covered by
  an explicit late-arrival test.
- [Transcript exceeds context for a single Haiku call] → distiller truncates
  per-session input deterministically (head+tail sampling) before the call;
  entries summarize days, not verbatim logs, so lossy input is acceptable.
- [Project renames/moves split fold keys] → accepted; entries are keyed by
  the path as archived. A rename produces separate entries per path, which is
  historically accurate.
- [engineering-notebook convergence] (their roadmap mentions a centralized
  agent log archive) → watch item on #12; no code impact now.

## Migration Plan

1. Migration `0002_journal_entries.sql` is additive (new table + indexes
   only); hub deploys with it via the standard staging → infra binary-swap
   path (`docs/archive/deployment.md`). Rollback = deploy previous binary;
   the unused table is inert.
2. Distiller ships behind its own plist (`dev.cchv.distiller`); installing it
   is a separate, reversible step. No entry generation happens until it runs.
3. Backfill runs attended, in chunks, after the forward path has proven
   itself for a few days.

## Open Questions

- None blocking. Deferred (tracked on #12): dropped-threads report shape;
  whether the webapp journal UI is worth building before pgvector semantic
  search lands.
