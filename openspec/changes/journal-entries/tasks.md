# Tasks: journal-entries

Contract-shaped: each group states what done looks like; the specs in
`specs/` are the acceptance authority. Rust tests run single-threaded
(`cargo test -- --test-threads=1`).

## 1. Schema

- [ ] 1.1 Write `migrations/0002_journal_entries.sql`: `journal_entries`
  table per the journal-entries spec (unique `(entry_date, project_path)`,
  `status IN ('entry','skip')`, headline/summary/topics/open_questions,
  session provenance, `model`, `generated_at`, `search_text` + generated
  tsvector + GIN index). Additive only — no existing table touched. Verify:
  migration applies cleanly on a database that already ran 0001.

## 2. Hub API

- [ ] 2.1 `GET /v1/journal/pending` (read-auth): data-derived work list —
  missing or dirty (session ingested after `generated_at`) closed groups,
  newest-first, with date lower bound + limit params; today (per
  `day_start_hour`) excluded. Tests: missing-entry pending, late-arriving
  session re-dirties, skip row suppresses until new data, open day excluded.
- [ ] 2.2 `POST /v1/journal/entries` (machine-token auth): validated upsert
  by `(entry_date, project_path)`. Tests: upsert replaces dirty entry without
  duplicates; invalid payloads (zero topics, unknown session id, bad status)
  rejected 4xx with no partial write.
- [ ] 2.3 `GET /v1/journal/entries` (read-auth): browse filterable by project
  + date range, newest-first, paginated, `entry` rows only by default. Test:
  project/date-range listing returns full entry content incl. session ids.
- [ ] 2.4 `/v1/search` `scope` param (`all`|`messages`|`journal`) + additive
  `journal` block ranked over entry text. Tests: default scope returns both
  blocks; **`scope=messages` response byte-compatible with the pre-change
  shape (regression anchor)**; `scope=journal` skips message search; skip
  rows never surface.

## 3. Distiller

- [ ] 3.1 `scripts/cchv-distill.py` (PEP 723/uv): drain pending → fetch
  session messages via hub browse API → build prompt (port
  engineering-notebook `src/summarize.ts` schema/prompt: headline, 2–5
  sentence summary, 3–8 topics, open_questions, SKIP sentinel) → one
  `claude -p --output-format json` call (model configurable, default
  Haiku-tier, single turn) → validate against entry schema → POST. Config:
  hub URL + token (bao-first, `kv/infra/cchv/hub-tokens`, op fallback,
  never prompt when non-interactive), `day_start_hour` (default 04:00),
  forward horizon. Deterministic head+tail truncation for oversized
  transcripts. Flags: `--dry-run` (generate + validate, no POST),
  `--backfill [--from DATE] [--limit N]` (newest-first, resumable).
  Verify: `--dry-run` against a fixture group produces a schema-valid entry;
  malformed LLM output is rejected locally and the group stays pending;
  two bounded backfill invocations continue without duplicates or gaps.
- [ ] 3.2 launchd plist `dev.cchv.distiller` (daily ~05:30 +
  `RunAtLoad`-style wake catch-up) conforming to launchd-resilience:
  `CCHV_NONINTERACTIVE=1`, ThrottleInterval, degrade-don't-loop. Document
  install + backfill runbook in `docs/archive/deployment.md` (new §
  alongside 3b).

## 4. Ship

- [ ] 4.1 Full quality gate (CLAUDE.md Phase 1: pnpm install, tsc, vitest,
  lint, cargo test --test-threads=1, clippy -D warnings, fmt --check,
  i18n:validate — frontend untouched but the gate is all-or-nothing).
- [ ] 4.2 E2E on m4m: deploy hub (staging → infra binary swap per
  deployment.md), install distiller plist, distill one real closed day,
  confirm the entry is returned by `/v1/search` (default scope) and
  `/v1/journal/entries`.
- [ ] 4.3 Release `cchv-v0.6.0` per CLAUDE.md release process (version bump,
  sync-version, tag, push internal+origin, CI release).
- [ ] 4.4 Close the loop: update issue #12 (link change, `status/ready` →
  done state per backlog flow); relay note to home-network that the hub
  gained the journal surface (their eval layer references it); follow-up
  items filed: cchv-find journal-first update (CONTEXT repo),
  dropped-threads report, webapp journal UI.
