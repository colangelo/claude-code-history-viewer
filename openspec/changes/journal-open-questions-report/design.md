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
- Claim nothing the data cannot support. In particular, never label a thread "resolved".
- Reuse the existing embedder and sweep, with no new model and no external call.

**Non-Goals**
- An explicit resolution signal from the distiller (Q1).
- Cross-project grouping. A thread lives in one project; identity-grouped projects
  (`project-identity`) count as one project, as the journal already treats them.
- Webapp UI (Q2).

## Decisions

### D1. Group by embedding similarity, per project, at request time

Store one vector per question (`journal_question_embeddings(entry_id, ordinal, model,
embedding)`), filled by the existing sweep as a new source. At request time, load the
window's vectors for the project and merge with single-link clustering at a cosine
threshold (start at 0.85, tuned in task 1.2), ordered by time. This is a few hundred
vectors at most, so it runs in memory in milliseconds and needs no index.

**Measured (task 1.2, 2026-10-09): embedding grouping alone is not good enough.** At the
only threshold with precision ≥ 0.95 (cosine ≥ 0.85), it finds about half of the restatements a
human marks and turns 107 questions into 97 threads. That is a negligible reduction, and every
missed merge would show a live thread as `quiet`. Questions that are related but different sit
at 0.72–0.84, right among the real restatements, so no threshold separates them. bge-small is a
retrieval model, and short same-project phrases all look alike to it.

**Revised recommendation: thread identity comes from the distiller (Q1 becomes the main path).**
When the distiller writes a day's entry, it already has the day's work in context. Give it the
project's open threads from the last N active days (about 30 questions × 61 characters, so under
1k extra prompt tokens per run) and have it output, for each question, either `continues
<thread-id>` or new, plus the threads the day **resolved**. That gives exact thread identity and
the resolution signal D2 couldn't derive. Embeddings stay useful only as a fallback merge at
≥ 0.85 for entries written before the change.

*Rejected:* exact or normalised-text dedup. The distiller rephrases the same thread daily,
so exact matching would almost never merge. *Rejected:* asking the model to group, which
costs model spend on every read.

### D2. Three states, all derivable

| State | Rule | What it does NOT mean |
|---|---|---|
| `recurring` | restated on ≥ 2 distinct days | that it is still open today |
| `quiet` | last restated before the project's N most recent active days (default N = 3) | that it was resolved: it may equally have been dropped or crowded out by the 5-question cap |
| `new` | seen once, within the last N active days | anything about importance |

"Active day" means a day with an `entry` row for the project. Measuring age in active days
rather than calendar days avoids marking every thread quiet after a holiday.

### D3. Endpoint shape

`GET /v1/journal/open-questions?project=<path|identity>&days=30&state=…`. The response
lists threads, newest `last_seen` first. Each thread has a representative phrase (the most
recent wording), first and last seen dates, occurrence count, state, and entry ids. Without
`project`, it returns per-project counts by state only. The endpoint uses the same read
auth as `/v1/journal`.

## Risks / Trade-offs

- **Threshold too loose or tight.** Merging two different threads hides one; failing to merge
  repeats one. *Mitigation:* task 1.2 hand-labels pairs from one project to pick the
  threshold, and the representative wording plus the entry ids keep every merge auditable.
- **bge-small on short phrases.** The vectors are tuned for retrieval, not paraphrase.
  *Mitigation:* the same labelled set measures it. If the precision is poor, fall back to
  stricter thresholds and accept more `new` duplicates. A wrong merge is worse than a
  duplicate.

## Open Questions (ac)

- **Q1 (now gating, see D1 "Measured").** Should the distiller get thread continuity and
  a resolution signal? Rec: **yes**. It is the only approach that measured well enough, and it
  costs under 1k prompt tokens per distill on the Codex path. It is a distiller prompt and
  schema change (`scripts/cchv-distill.py` + the journal POST), and the distiller is an
  installed copy, so it ships only through a release plus an infra reinstall.
- **Q2.** Webapp surface: a "Threads" panel in the Journal tab? Rec: later, endpoint and
  `cchv-find` first.
