# hub-stats-duckdb-mirror

## Why

`GET /v1/stats/global` takes **18.0 s** in production. That is not an edge case:
the webapp's Analytics tab sends a 30-day window on open, and 2.25M of the
archive's 2.53M messages fall inside 30 days, so the *default* view pays the
full price. Every visit to the tab costs 18 seconds.

The endpoint is functionally correct and gate-verified (Gitea #24, shipped in
`hub-analytics`). The problem is that it runs an OLAP workload — eight aggregate
scans over 2.5M rows, dominated by `count(DISTINCT …)` and `GROUP BY` — against
a row-store. **Three optimizations have now been tried and refuted**, each
measured rather than argued:

1. Dedup once per request instead of per rollup — 30.9 s → 13.9 s. *Real, kept.*
2. Folding the three tool queries into one pass — 13.9 s → 13.7 s. **No-op.**
3. `SET LOCAL work_mem` to stop the dedup sort spilling — **~940 ms, 7%**
   (measured 2026-07-25: 3.43 s → 2.48 s, external merge → in-memory quicksort,
   at ~525 MB transient). It also caps option 2 of #24, the expression index:
   both target only materialization, which is 3.5 s of 13.7 s, so even driving
   it to zero leaves ~10 s.

Measuring the same eight rollups on a DuckDB copy of the same data settles it.
Provenance, so the table is not over-read: the **per-statement Postgres figures
are from the `hub-analytics` profiling** recorded in that change; the
materialization row, the endpoint total, and every DuckDB figure were measured
on 2026-07-25. The archive also grew between the two sessions (2.53M → 2.80M
stored rows), which moves the Postgres numbers slightly *against* this change,
not for it.

| statement | Postgres (pg1) | DuckDB | |
|---|---|---|---|
| materialize scope | 3.43 s | 1.79 s | 1.9× |
| daily | 2.20 s | 0.081 s | 27× |
| totals | 1.30 s | 0.010 s | 130× |
| providers | 1.27 s | 0.012 s | 106× |
| top projects | 1.24 s | 0.011 s | 113× |
| heatmap | 0.97 s | 0.058 s | 17× |
| tools/skills/subagents | 0.70 s | 0.033 s | 21× |
| models | 0.26 s | 0.006 s | 43× |
| **endpoint** | **13.7 s** (18.0 s live) | **2.21 s** | **6.2×** |

The aggregates alone collapse from ~10 s to **0.42 s**. And the residual 1.79 s
is the dedup window function, which does not have to be paid per request: with
`usage_row`/`conversational` computed once at refresh, the measured per-request
cost over the real 30-day window is **~0.39 s**.

This is why the change is *subtractive*. A cache — snapshot table,
stale-while-revalidate, background refresher, a relative `window` param to dodge
the midnight cache-key cliff — was designed and then discarded, because all of it
existed only to hide an 18 s computation. At 0.4 s you simply run the query.

## What Changes

- The hub gains a **derived read model**: a DuckDB file holding a narrow
  projection of `messages`, `sessions`, `projects` and both tool tables
  (size unverified: the 119 MB originally recorded here is probably an order of
  magnitude low — measured for real in task 6.5), refreshed incrementally from
  Postgres.
- `/v1/stats/*` reads only from the mirror. The Postgres rollups are **replaced,
  not kept as a fallback** — two ports of eight rollups would drift.
- A background refresher appends new rows on an interval — idempotently, over an
  overlap window behind the watermark, so rows whose transactions commit out of
  order are never silently skipped — and computes the two derived booleans for
  the new rows only.
- Incremental refresh covers **inserts only**. Operations that UPDATE mirrored
  columns in Postgres (`hub backfill-analytics` is the live example) require a
  mirror rebuild: a new `hub mirror rebuild` subcommand builds the file aside
  and swaps it in without interrupting serving.
- While no mirror exists, `/v1/stats/*` returns `503` with `Retry-After`. The
  file survives restarts, so only the first start after deploy pays the build.
- Mirror staleness is reported on **response headers**, and a new
  `GET /v1/healthz/stats` reports age **and watermark lag against Postgres**, so
  completeness is monitorable, not just recency.
- `GET /v1/stats/sessions/{id}` accepts a session UUID as well as the numeric row
  id, matching `GET /v1/sessions/{id}/messages` (Gitea #26).

Postgres remains the system of record. Ingest, search, journal and embeddings are
untouched, and the mirror is derived state that can be deleted and rebuilt at any
time.

## Non-goals

- Precomputed rollup tables (option 3 of #24) — the measurement makes them
  unnecessary.
- `SET LOCAL work_mem` — refuted above, and moot once Postgres leaves the read
  path.
- Any change to the `history-core` stat types, the response bodies, or ingest.

## Dependencies and ordering

**This change must land before `#23` (drop the Tauri desktop).** Its correctness
gate is the desktop-analytics oracle harness built in `hub-analytics` task 6.1,
and Deliverable 2 deletes that oracle. Once the desktop is gone there is nothing
independent left to diff a reimplementation of the rollups against.

Closes Gitea #24 and #26. (#27 was investigated during this work and closed as
correct behaviour: the 0% success rate on `Skill(read)`/`Skill(bash)` is real —
every one returns `<tool_use_error>Unknown skill: read</tool_use_error>`.)
