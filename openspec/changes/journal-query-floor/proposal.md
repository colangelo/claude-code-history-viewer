## Why

`GET /v1/healthz/journal` is monitor-polled and takes **3.7 s**; `GET /v1/journal/pending`
takes 3.25 s. Both are bounded below by the same thing: the shared logical-day fold
(`SESSION_DAYS_CTE`) reads **1,329 MB of heap across 170,151 pages** for the 7-day window
in order to extract **three narrow columns** — `messages."timestamp"`, `session_id`,
`created_at`. It never touches `content` or `raw`, yet every plan available to it today
visits the heap and pays for them. The endpoint has already timed out at the proxy once
and answered 502 (2026-08-21, 01:29Z, when the two-pass form took 6.9 s), and a health
check that flaps is worse than one that is slow, because the flap is what gets
investigated (#36).

Separately, **91 % of archived rows are not conversation at all** — they are Claude Code
sidecar state records with `content IS NULL` (`permission-mode`, `agent-color`,
`worktree-state`, …). Every count taken off `messages` overstates conversation by ~11×,
nothing in the schema or the API says so, and it is why roughly half of any journal
backfill legitimately posts `skip` (#41).

## What Changes

- **A covering index for the fold**, built once and owned by the deploy rather than by
  hub startup:
  ```sql
  CREATE INDEX CONCURRENTLY messages_journal_fold_idx
      ON messages ("timestamp", session_id) INCLUDE (created_at);
  ```
  It carries exactly what the fold reads, so the window can be served by an **index-only
  scan** — ~96 MB of index instead of 1,329 MB of heap, a ~14× reduction in bytes read.
  **No query text changes and no behaviour changes**; this is a plan change only.
- **Deliberately NOT partial.** A `WHERE content IS NOT NULL` variant would index ~8.9 %
  of rows (~27 MB instead of ~300 MB) but would make the fold blind to days composed
  purely of state records, so those days would stop being pending and would never receive
  their `skip` row. That is a **behaviour change wearing a performance change's clothes**,
  and this change refuses it. Whether such days deserve a journal row at all is a real
  question — it is left open as #41, decoupled.
- **The row-count caveat gets documented where counts are quoted**, so "N messages" stops
  being read as "N conversation turns". This is documentation, not behaviour, so it is
  tasks rather than a spec delta — exposing a conversation-only count in analytics is a
  real API change and is deliberately left to #41 rather than smuggled in here.
- **An autovacuum assertion**, because an index-only scan is only as good as the
  visibility map on an append-heavy table. If the map is not being maintained the plan
  silently degrades to an index scan with heap fetches and the win evaporates — measured
  and asserted rather than assumed.

Not in scope: changing what the fold means, filtering state records at ingest (#41,
irreversible, explicitly deferred), and `SET LOCAL random_page_cost` (kept as the
zero-storage fallback if the index is rejected — it can only ever buy the bitmap path,
~2–3×, never the index-only path).

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `journal-health`: the staleness endpoint's evaluation SHALL be served without reading
  message payloads — a performance floor stated as a property of the data the check is
  allowed to touch, not as a wall-clock number.
- `journal-entries`: the catch-up pending work list SHALL be subject to the same floor
  (it runs the identical fold) and the access path SHALL NOT change which groups it
  returns.

## Impact

- **Schema**: one new index. **Not** a startup migration — `CREATE INDEX CONCURRENTLY`
  cannot run inside a transaction, and `0006` already cost 6.66 s at hub startup for a
  smaller index. It is a relayed deploy step (`docs/archive/deployment.md` §1), applied
  before the release that depends on it.
- **Query code**: none. `SESSION_DAYS_CTE` is unchanged; the planner picks the new path.
- **Storage**: ~300 MB added to a 10 GB table.
- **Verification**: `EXPLAIN (ANALYZE, BUFFERS)` on both endpoints' queries before and
  after, asserting the plan is an index-only scan and that `Heap Fetches` is small.
- **Docs**: `docs/archive/deployment.md` (the index build + its measurements),
  and wherever the archive quotes message counts.
