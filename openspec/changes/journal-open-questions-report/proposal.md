# Proposal

## Why

Since v0.6.0 every journal entry records `open_questions`: the threads a day's work left
unresolved. Nothing reads them back. On 2026-10-09 (read-only count on pg1) the archive held
**2,462** of them across 652 entries and 181 projects, and **935** from the last 30 days
across 52 projects; 13 projects had 20 or more each. That is too many to scan by hand, and the
same thread is restated in different words on consecutive days. The questions exist so that
dropped work resurfaces, and today it doesn't (Gitea #15).

## What Changes

ac decided design.md Q1 on 2026-10-10 (relayed by manager-lab): **the distiller links new open
questions to earlier ones and marks the ones a day resolved.** Task 1.2 had shown that embedding
similarity cannot do the grouping, so thread identity comes from the distiller, which already has
the day's work in context.

- The distiller fetches the project's recent unresolved threads before it writes an entry and
  gets them in the prompt (under 1 k extra tokens). For each open question it answers
  `continues <thread id>` or new, and it lists the threads the day resolved.
- `journal_entries` gains `open_question_threads BIGINT[]` (one thread id per open question)
  and `resolved_threads BIGINT[]`. The hub assigns ids to new threads and rejects ids that are
  not open questions of the same project. A re-distill replaces both, like every other field.
- The migration gives every existing open question its own thread id, so new questions can be
  linked to them from the first run. No re-distillation of old days.
- New read endpoint `GET /v1/journal/open-questions`: per project, threads with latest wording,
  first and last seen, occurrences, entry dates and a state: `resolved`, `open` or `quiet`
  (unresolved, not restated in the project's last N active days). The distiller and people use
  the same endpoint.
- **Not in this change:** a webapp panel (Q2); retro-merging restatements written before the
  change (each stays its own thread).

## Capabilities

### New Capabilities

- `journal-open-questions`: grouping journal open questions into threads per project, and
  serving them with a state that claims no more than the archive knows.

### Modified Capabilities

- `journal-entries`: the write endpoint accepts and validates thread links and resolutions,
  and the distiller is given the project's open threads and asked to link and resolve them.

## Impact

- **Hub:** migration `0012`, the POST validation and upsert in `journal.rs`, and the new
  endpoint with read auth like the other `/v1/journal` reads.
- **Distiller:** prompt, output parsing and validation in `scripts/cchv-distill.py`. It is an
  installed copy, so it ships only through a release plus an infra reinstall.
- **Daemon, webapp:** unchanged.
- **Docs:** `cchv-find` skill section (CONTEXT, relayed to its owner).
