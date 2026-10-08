# Proposal

## Why

Since v0.6.0 every journal entry records `open_questions`: the threads a day's work left
unresolved. Nothing reads them back. On 2026-10-09 (read-only count on pg1) the archive held
**2,462** of them across 652 entries and 181 projects, and **935** from the last 30 days
across 52 projects; 13 projects had 20 or more each. That is too many to scan by hand, and the
same thread is restated in different words on consecutive days. The questions exist so that
dropped work resurfaces, and today it doesn't (Gitea #15).

## What Changes

- New hub read endpoint `GET /v1/journal/open-questions`. It returns, per project, the open
  questions from a time window, **grouped into threads**: near-duplicates across days merge
  into one thread carrying first seen, last seen, occurrences and the entry ids behind it.
- Each question gets an embedding from the model the hub already runs for journal search
  (bge-small-en-v1.5). Grouping is cosine similarity over those embeddings within one
  project. The existing embed sweep fills them; no new model and no external call.
- Each thread gets a **state** derived only from data the archive has:
  - `recurring`: restated on two or more days;
  - `quiet`: not restated since the project's last N active days, so possibly resolved and
    possibly dropped. The archive can't tell which, and the report says so.
  - `new`: seen once, recently.
- `cchv-find` documents the endpoint as the way to answer "what did I leave hanging in
  <project>?".
- **Not in this change:** an explicit "resolved" signal. That needs the distiller to compare a
  day's work against earlier questions: a prompt change with per-run model cost, ac's
  decision (design.md Q1). No webapp UI yet either (Q2).

## Capabilities

### New Capabilities

- `journal-open-questions`: grouping journal open questions into threads per project, and
  serving them with a state that claims no more than the archive knows.

### Modified Capabilities

None. `journal-entries` already requires that `open_questions` be stored. This change only
reads them. The embed sweep gains a source but its requirements are unchanged.

## Impact

- **Hub:** a migration for `journal_question_embeddings`, a `embed_sweep.rs` source, the
  endpoint and the grouping in `journal.rs` (or a new module), plus read-auth like the other
  `/v1/journal` reads.
- **Load:** about 2.5 k short strings to embed once, then a few per distiller run. Grouping
  works on one project's window, at most a few hundred vectors, in memory per request.
- **Distiller, daemon, webapp:** unchanged.
- **Docs:** the `cchv-find` skill section (CONTEXT; relayed to its owner if needed).
