## Context

See `proposal.md` — *Why*. The measurements this design rests on were taken on the live
archive (pg1) on 2026-08-21 and are recorded on **#36 comment 7390** and **#41 comment
7384**; the numbers below are quoted, not re-derived.

- `SESSION_DAYS_CTE` (`crates/hub/src/journal.rs`) is shared verbatim by
  `healthz_journal` and `journal::pending` — deliberately, so the two cannot disagree
  about which sessions belong to a day. It selects `m."timestamp"`, `m.session_id`,
  `m.created_at`, joins `sessions`, groups by day/project/session. **It reads no payload
  column.**
- Existing indexes on `messages`: `("timestamp")` 235 MB, `(session_id, "timestamp")`
  228 MB (= 32 B/row over 7.5 M rows), plus the FTS GIN 474 MB and others.
- The 7-day window is 2,412,130 rows across **170,151 heap pages = 1,329 MB**.
- Row kinds are physically interleaved: 81 % of the pages holding a conversation row also
  hold a state record, and conversation rows occupy 35 % of pages archive-wide.

## Goals / Non-Goals

**Goals:**
- Remove the payload read from the fold's cost, for both endpoints, without touching the
  fold's text or its meaning.
- Leave the deploy able to *prove* the win rather than assume it.

**Non-Goals:**
- Any change to which groups the fold returns. If a plan change alters the result set, it
  is the wrong plan change.
- Deciding whether state-record-only days deserve a journal row (#41).
- Ingest-side filtering (irreversible; explicitly deferred).
- A wall-clock SLO. The endpoint's speed is a consequence here, not the contract.

## Decisions

1. **A covering index, not a partial one.**
   `("timestamp", session_id) INCLUDE (created_at)`. The leading column is `"timestamp"`
   because the window predicate is a range on it; `session_id` follows because the fold
   groups by it; `created_at` rides in `INCLUDE` because `max(created_at)` is needed but
   never searched — putting it in `INCLUDE` keeps it out of the B-tree's comparison path
   while still making the index cover the query.
   *Alternative rejected:* `WHERE content IS NOT NULL`. It is ~11× smaller (~27 MB) and it
   changes behaviour — the fold would stop seeing days made only of state records, so
   those days would never be pending and never receive a `skip` row (~half of backfill
   outcomes). A speed change that silently withdraws a row from the journal is not a speed
   change. This is the single most important decision here and it is the one the issues,
   read separately, would have got wrong.
   *Alternative deferred:* `SET LOCAL random_page_cost`. Zero storage, but it can only
   coax the planner onto the **bitmap** path (2–3×, measured), never the index-only path
   (~14×), because no existing index carries `created_at`.

2. **The index correction matters and is worth stating.** #36's own option 2 proposed
   `on ("timestamp", session_id)` — without `created_at`. That index cannot serve the
   fold: `max(m.created_at)` forces a heap visit per row, so the index-only scan never
   materialises and the win collapses back to the bitmap case. The `INCLUDE` is the whole
   difference between ~2–3× and ~14×.

3. **Built by the deploy, not by hub startup.** `CREATE INDEX CONCURRENTLY` cannot run
   inside a transaction, which is how `sqlx` runs migrations; and even a plain
   `CREATE INDEX` here would block writes on a 7.5 M-row table at every hub start. `0006`
   cost 6.66 s at startup for a much smaller index and that was already noted as a
   startup window to plan around. So: an operator step in `docs/archive/deployment.md`,
   relayed to infra, applied **before** the release that expects it. The hub needs no
   code change, so a hub running against a database without the index is simply the
   status quo — which makes the ordering forgiving.

4. **Verification is a plan assertion, not a stopwatch.** `EXPLAIN (ANALYZE, BUFFERS)` on
   both queries, asserting: node is `Index Only Scan using messages_journal_fold_idx`,
   `Heap Fetches` is small relative to rows, and `shared read+hit` blocks drop by roughly
   an order of magnitude. A timing alone would be confounded by cache state and by
   whatever else pg1 is doing; the buffer counts are the thing the index actually changes.
   Take the "before" reading in the same session as the "after", or it is not a comparison.

5. **Autovacuum is part of the design, not an afterthought.** An index-only scan reads
   the visibility map to decide whether it may skip the heap; on an append-heavy table
   whose autovacuum is behind, `Heap Fetches` rises and the plan quietly degrades toward
   the thing it replaced. So the change asserts the current state of
   `pg_stat_user_tables` for `messages` (last autovacuum, dead tuples) and records it, so
   a future regression has a baseline to be compared against rather than a shrug.

## Risks / Trade-offs

- [The planner declines the new index] → the same cost model that prefers a seq scan today
  might prefer one still. Mitigation: the index makes an index-**only** scan available,
  which is a materially cheaper node than the bitmap path it already rejects, so the cost
  estimate changes in kind rather than in degree. Fallback if it still declines: decision
  1's deferred alternative (`SET LOCAL random_page_cost` on just these two statements),
  and the index remains useful to the bitmap path regardless. **This is why the tasks
  measure before committing to a release.**
- [~300 MB of storage] → on a 10 GB table with 209 GB free on the host. Cheap, and stated
  so nobody has to re-measure headroom.
- [`CREATE INDEX CONCURRENTLY` fails and leaves an INVALID index] → it is the documented
  failure mode; the recovery (`DROP INDEX CONCURRENTLY`, retry) goes in the deploy step
  rather than being discovered live.
- [Write amplification on ingest] → one more index to maintain on the hottest table.
  Ingest is batched and append-only, so the cost is per-row and small, but it is real and
  should be watched on the first heavy ingest after the build.
- [The measurements age] → they are a snapshot of a growing archive. The spec is written
  as "does not read payloads", which stays true as the table grows; the numbers in the
  docs are dated.

## Migration Plan

1. Land the spec deltas and docs; no code change is required, so nothing ships broken.
2. Relay the index build to infra: `CREATE INDEX CONCURRENTLY` against pg1, off-peak,
   with the `DROP INDEX CONCURRENTLY` recovery named. Capture `EXPLAIN (ANALYZE, BUFFERS)`
   before and after in the same session.
3. If the plan flips to an index-only scan with small `Heap Fetches`: record the readings,
   close #36. If it does not: keep the index (it still helps the bitmap path), fall back to
   decision 1's deferred alternative, and say so on the issue rather than quietly leaving
   the endpoint slow.
4. Rollback: `DROP INDEX CONCURRENTLY messages_journal_fold_idx`. Nothing depends on it —
   no query names it and no code references it.
