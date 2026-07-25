# hub-stats-duckdb-mirror — design

Where a decision rests on a number, the number is here.

**Provenance of the figures.** Everything below was measured on 2026-07-25
against live pg1 and against a DuckDB copy of the same data, with one exception
named in `proposal.md`: the per-statement Postgres breakdown (daily, totals,
providers, top projects, heatmap, tools, models) is carried over from the
`hub-analytics` profiling. The archive held 2.53M stored rows then and 2.80M
now, so those carried-over figures understate today's Postgres cost — the
comparison is conservative, not flattering.

## Architecture

```
ingest ──▶ Postgres (system of record; unchanged)
                │
                │  refresher, every N minutes:
                │  SELECT … WHERE id > max_id   ← incremental, append-only
                ▼
          DuckDB mirror  (~/.config/cchv/stats-mirror.duckdb, 119 MB)
          • narrow projection + usage_row / conversational, computed at refresh
                │
                ▼  ~0.4 s
          GET /v1/stats/{global, projects/{key}, sessions/{id}}
```

The mirror holds a 13-column projection of `messages` (only the columns the
rollups touch), `sessions(id, project_id)`, `projects`, and both tool tables
whole (~137k rows each). It is derived state: deletable and rebuildable at any
time, and it never becomes an authority for anything.

## D1 — Why a second engine at all, given the single-store principle

The archive's design principle is that Postgres is the one place data lives, and
this change bends it. It is justified because the alternative designs were
measured and are worse, not because columnar engines are fashionable:

- Options 1 and 2 of #24 (`work_mem`, expression index) both target only
  materialization — 3.5 s of 13.7 s. Their combined ceiling leaves ~10 s.
- Option 3 (precomputed rollup tables) reaches sub-second but introduces a
  refresh strategy over *aggregates*, where dedup makes incremental maintenance
  subtle, plus a timezone-bucketing problem (UTC hour buckets re-bucket exactly
  only to whole-hour offsets).
- A cache reaches "instant on repeat" but never "instant", and needs
  invalidation, warming, and an API change to keep its keys stable.

The mirror is the only option that makes the computation itself fast, which is
why it *removes* machinery instead of adding it. The staleness and sync concerns
it introduces are the same two the cache design introduced — not a new class of
risk.

## D2 — Incremental refresh is append-only, and provably so

`usage_row` means "is this the lowest-id row of its dedup group", where the group
is `(session_id, COALESCE(message_id, uuid, id::text))`.

Postgres ids are monotonic. An appended row therefore can never become the
minimum of a group that already exists, so **appending never invalidates a
previously computed `usage_row`**. Refresh is a pure append plus a computation
over only the new rows; no history is recomputed and there is no invalidation
path to get wrong.

This holds for the awkward case too. Time Machine backfill inserts rows with
*old* timestamps under *new* ids; such a row joins its group with a higher id and
correctly reads `false`. The rule is "lowest id seen so far", which is exactly
what the current Postgres implementation computes over whatever is in the table.

## D3 — Compute `usage_row` globally, not per window

Today the date window is applied *before* dedup, so `usage_row` is relative to the
window. Persisting it means computing it once over the whole archive, which
differs only for a logical message whose rows straddle the window boundary.

Measured: of **2,648,869** dedup groups, 91,526 are multi-row and
**10 span a date boundary** — 0.0004%. And a boundary only matters if the window
edge falls exactly there. This is a footnote, not a divergence; it is recorded
here so a future reader does not rediscover it as a bug.

## D4 — Replace the Postgres rollups; do not keep them as a fallback

A fallback would mean a cold or broken mirror degrades to the old 18 s answer
instead of a `503`. Rejected: it means maintaining two ports of eight rollups
forever, they will drift, and the oracle gate would have to cover both. The
mirror file survives restarts, so the unavailable window is the first start after
deploy and nothing else.

## D5 — Staleness on headers, not in the body

`X-Stats-Mirror-Refreshed-At` and `X-Stats-Mirror-Age-Seconds` on every
`/v1/stats/*` response.

Deliberate: adding a field to the `history-core` stat types breaks **struct
literals** in `src-tauri` even when serde-compatible, and `rust-tests.yml` builds
that crate — a trap that cost real time during `hub-analytics`. A header sidesteps
it, and stays valid after Deliverable 2 deletes the desktop.

## D6 — The refresher must stay out of the credential watchdog's path

`hub-credential-resilience` exits the process after three consecutive `28P01`
rejections so launchd re-resolves a rotated password. The refresher talks to
Postgres too, and must not participate in that: a failed refresh leaves the
mirror exactly as it was and keeps serving with the age header climbing. It never
exits, never counts strikes, and its errors are logged distinguishably so a
refresh failure is not read as a credential failure.

## Error handling

| Condition | Behaviour |
|---|---|
| No mirror yet | `503` + `Retry-After`; build runs in background |
| Mirror corrupt | move aside timestamped (never delete), rebuild, log loudly |
| Postgres unreachable at refresh | keep serving; age header climbs; no exit |
| Refresh overruns its interval | single-flight; skip the tick |
| Disk full | refresh fails, keep serving, log |

Silent staleness is the one genuinely new failure mode, so it gets
`GET /v1/healthz/stats` — unauthenticated, Gatus-consumable, mirroring
`/v1/healthz/journal`'s shape.

## Testing

**The oracle gate is load-bearing.** `hub-analytics` task 6.1 built a harness
that diffs hub output against the desktop analytics field by field; it caught two
real bugs. Re-point it at the DuckDB implementation and require the same verdict:
exact match on token, cost, message, session and activity fields, with the two
documented divergences still one-directional (hub success rate ≤ oracle's, hub
tool counts ≤ oracle's).

This is why the change must precede #23 — Deliverable 2 deletes the oracle.

Beyond the gate: the existing endpoint tests port onto the new path with the same
fixtures and expectations (no second implementation retained); a new
incremental-refresh test appends both a row joining an existing dedup group and a
backfilled row carrying an old timestamp under a new id, asserting D2; and a
cold-start test asserts `503` then `200`. CI needs no new services —
`archive-tests.yml` already runs Postgres, which is what the mirror reads *from*.

## Risks

| Risk | Handling |
|---|---|
| Silent staleness | age headers + `/v1/healthz/stats` + a Gatus check relayed to infra |
| Binary size / CI build time | bundled DuckDB grows the ~15 MB hub binary; confirm the macos-14 release job still fits before tagging |
| Disk on m4m | 119 MB today, grows with the archive — headroom check goes in the deploy relay |
| DuckDB memory on a shared box | set an explicit `memory_limit`; m4m also runs the distiller and daemon |
| Port fidelity | the oracle gate |

## Deployment note

After the §2b binary swap, `/v1/stats/*` returns `503 warming` for roughly four
minutes while the first mirror builds (the initial pull from pg1 measured 227 s,
network-bound). **This is expected and is not a rollback trigger** — it belongs in
the relay text, since an unexplained 503 after a swap is exactly the shape that
has caused false alarms before.
