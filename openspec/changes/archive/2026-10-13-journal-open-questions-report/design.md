# Design

## Context

`journal_entries` (migration 0002) keeps one row per (date, project) with `status`
`entry` or `skip`, `open_questions TEXT[]` (0–5 phrases, about 61 characters on average, at
most 205), and the session ids distilled. The hub already embeds each entry's text with a
local candle bge-small-en-v1.5 model into `journal_embeddings` (migration 0004), filled by
`embed_sweep.rs`. The distiller writes `open_questions` once per entry and never looks back.

Measured 2026-10-09 (read-only counts, pg1): question counts per entry are 0 → 565 entries,
1 → 26, 2 → 59, 3 → 119, 4 → 129, 5 → 289. The cap of 5 binds on the busiest days, so on
those days a question may simply be crowded out. That's one more reason "not restated" can't
be read as "resolved".

## Goals / Non-Goals

**Goals**
- Turn hundreds of daily phrases into a short list of threads per project.
- A thread's state is what the record says: resolved only when a distilled day said so.
- No new model, no extra model call: the links come out of the distill call that already runs.

**Non-Goals**
- Cross-project threads. A thread lives in one `project_path`, the key the journal already uses.
- Merging restatements written before the change: each existing question becomes its own thread.
- Webapp UI (Q2).

## Decisions

### D1. Thread identity comes from the distiller (ac, Q1, 2026-10-10)

**Measured first (task 1.2, 2026-10-09): embedding grouping cannot do this.** At the only
threshold with precision ≥ 0.95 (cosine ≥ 0.85), bge-small finds about half of the
restatements a human marks and turns 107 questions into 97 threads. Related-but-different
questions sit at 0.72–0.84, among the real restatements, so no threshold separates them.

So the distiller decides. Before writing a day's entry it fetches the project's unresolved
threads (D3) and the prompt lists them as `T<id> (last seen <date>): <latest wording>`. Its
JSON gains, per open question, `"continues": <id or null>`, and a top-level `"resolved": [ids]`
for threads the day's work settled. The distiller drops ids it did not offer and any id that is
both continued and resolved, so a model slip turns into a new thread, never a rejected entry.

*Rejected:* embedding similarity (measured above); asking a model to group at read time
(model spend on every read); exact-text dedup (the distiller rewords daily).

### D2. Storage: two aligned arrays on the entry row, plus a sequence

`journal_entries` gains `open_question_threads BIGINT[]` (same length and order as
`open_questions`) and `resolved_threads BIGINT[]`, and the hub owns `journal_thread_id_seq`.
On POST the hub assigns a fresh id to every question without one, checks that every given id
already appears in `open_question_threads` of **another** entry of the same `project_path`,
rejects an id both continued and resolved, and upserts the arrays with the rest of the row.

Why not tables: a re-distill replaces the whole row today, and keeping links in the row keeps
that true. A day's links and resolutions are rewritten with it, and nothing can be orphaned.
A thread is just the set of questions that carry its id.

Migration `0012` seeds ids for every existing question (one thread each), so the distiller can
link to them from the first run. About 2.5 k ids on 652 rows; skip rows get empty arrays.

### D3. States and the read endpoint

`GET /v1/journal/open-questions?project=<path>&days=30&state=open|quiet|resolved`, with the
read auth of the other `/v1/journal` reads; the machine token the distiller holds passes it.
Per project it returns threads, newest last seen first: thread id, latest wording, first and
last seen, number of days mentioned, entry dates, state and the date it was resolved.

| State | Rule |
|---|---|
| `resolved` | an entry of the project lists it in `resolved_threads`, dated after its last mention |
| `quiet` | unresolved, and last mentioned before the project's N most recent active days (N = 3) |
| `open` | unresolved and mentioned within them |

A mention after a resolution reopens the thread. "Active day" is a day with an `entry` row for
the project, so a holiday does not age threads. `quiet` still says what the old design said:
possibly dropped, possibly crowded out by the 5-question cap. Without `project` the endpoint
returns per-project counts by state.

The distiller asks for `state=open,quiet` within 30 days **before** the entry date, capped at
30 threads, newest first.

## Risks / Trade-offs

- **The model links wrongly.** A wrong `continues` merges two threads; a wrong `resolved` hides
  one. *Mitigation:* every link is auditable (entry dates per thread), a later mention reopens
  a resolved thread, and the hub refuses ids from other projects or that never existed.
- **Prompt cost.** At most 30 × ~61 characters, under 1 k tokens per distill; accepted by ac.
- **Backfill order.** A backfill run can distill an older day after newer ones. The offered
  threads are always those before the entry date, so links point backwards in time; a
  resolution recorded by an older day can be overtaken by a newer mention, which reopens.

## Open Questions (ac)

- ~~Q1~~ Decided 2026-10-10: yes, the distiller links and resolves.
- **Q2.** Webapp surface: a "Threads" panel in the Journal tab? Rec: later, endpoint and
  `cchv-find` first.
