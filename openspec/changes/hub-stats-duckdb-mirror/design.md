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
                │  SELECT … WHERE id > watermark − overlap   ← incremental, idempotent
                ▼
          DuckDB mirror  (~/.config/cchv/stats-mirror.duckdb, 119 MB)
          • narrow projection + usage_row / conversational, computed at refresh
                │
                ▼  ~0.4 s
          GET /v1/stats/{global, projects/{key}, sessions/{id}}
```

The mirror holds a 13-column projection of `messages` (only the columns the
rollups touch), `sessions` including the provider session identifier, `projects`
plus the project-identity tables, and both tool tables whole (~137k rows each).
The projection is deliberately wide enough that **every identifier a stats
endpoint accepts resolves from the mirror alone** — session row id, session
UUID, identity key — otherwise "stats survive a Postgres outage" would quietly
hold for only one of the three endpoints. It is derived state: deletable and
rebuildable at any time, and it never becomes an authority for anything.

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

## D2 — Incremental refresh: append-only for inserts; updates are out of contract

`usage_row` means "is this the lowest-id row of its dedup group", where the group
is `(session_id, COALESCE(message_id, uuid, id::text))`.

Postgres ids are monotonic, so a newly **inserted** row can never become the
minimum of a group that already exists — appending never invalidates a
previously computed `usage_row`. Time Machine backfill (old timestamps under new
ids) is covered by the same argument: such a row joins its group with a higher
id and correctly reads `false`.

But the naive reading of "append what's new" — `WHERE id > max_id`, advance the
watermark, done — is wrong in two ways, both caught in spec review:

**Out-of-order commit visibility.** Ids are allocated at insert time, but
transactions commit out of order, and several daemons plus the distiller write
concurrently. A refresh that runs while a lower-id transaction is still
uncommitted records a watermark above those rows; when they commit they are
below the watermark and would be skipped **forever, silently** — the exact
silent-staleness failure this design claims to guard against, invisible to an
age-based health check. The refresh therefore re-scans an **overlap window**
behind the watermark (by id, or by ingest `created_at` over the last several
minutes), and mirror inserts are **idempotent** (`INSERT OR IGNORE` on the
primary key), which makes the overlap free and leaves the group-minimum argument
untouched. The health endpoint additionally reports watermark lag (mirror max id
vs Postgres `max(id)`), so a stuck or lagging watermark is observable.

**Updates.** Append-only is sound for inserts only, and Postgres-side UPDATEs of
mirrored columns are not hypothetical: `hub backfill-analytics` — run against
production this very morning — is an `UPDATE messages SET message_id = …` over
existing rows. After such an update, Postgres regroups those rows under a shared
`message_id` while the mirror still holds them as singleton groups, and the
mirror **over-counts tokens** — precisely the bug this capability exists to
prevent. Chasing row-level updates is not worth the machinery. Updates are
instead **out of contract**: any operation that updates mirrored columns in
Postgres requires a mirror rebuild; `hub mirror rebuild` exists for exactly
that, builds the new file aside while stats keep serving from the old one, and
swaps atomically. The runbook note rides next to the `backfill-analytics`
instructions so the two operations travel together.

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

The deletion is gated, though. While both implementations still exist, they run
against the same live data and their outputs are diffed field by field — the
**differential gate**: exact equality expected everywhere, except that windowed
comparisons may differ for the ≤10 boundary groups D3 measures. A differential
against the very implementation being replaced catches porting errors more
directly than the desktop oracle can (the oracle exists to catch semantic drift
against an *independent* implementation, and still runs). Sequence:
differential gate → delete the Postgres rollups → oracle gate.

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

## D7 — Timezone bucketing is done in Rust, because `AT TIME ZONE` is not in DuckDB core

**This decision replaces an earlier one that was wrong, and the way it was wrong
is worth keeping.** Task 1.1 concluded that `LOAD icu` fails but that IANA
timezone support is in DuckDB core, so the port could use `AT TIME ZONE`
directly. Re-probed on 2026-07-25 against a **clean extension directory** with
`autoinstall_known_extensions=false` and `autoload_known_extensions=false`:

```
AT TIME ZONE 'Europe/Rome'   FAIL  Extension Autoloading Error: ... required extension 'icu'
AT TIME ZONE 'UTC'           FAIL  Extension Autoloading Error: ... required extension 'icu'
extract(dow|hour FROM ts)    OK
date_part('day', interval)   OK
strftime / casts / epoch     OK
lag() / bool_or / LATERAL    OK
```

`AT TIME ZONE` needs `icu` for **every** zone, UTC included. The original probe
passed because `~/.duckdb/extensions/v1.5.5/osx_arm64/icu.duckdb_extension` had
been downloaded onto the probing machine (timestamped 18:59 that evening, after
the `LOAD icu` check), and DuckDB silently autoloads a locally cached extension.
The lesson generalizes: **the `duckdb` CLI autoloads extensions the statically
bundled library does not, so SQL verified interactively can still fail inside
the hub.** `crates/hub/tests/duckdb_capability_test.rs` now asserts the whole
function inventory the rollups depend on, so a future DuckDB bump that moves one
behind an extension fails a test instead of a production endpoint.

Static linking was checked and is not available: the `duckdb` crate's `icu`
feature pulls in `bundled-cmake`, which requires a duckdb-rs *git checkout with
DuckDB sources* and fails from the crates.io package.

So the timezone offset is computed **in Rust, with `chrono-tz`** — IANA data
compiled into the binary, no extension, no runtime download, one deploy
artifact, and no coupling between the hub's DuckDB version and a separately
shipped `.duckdb_extension` file.

The mirror stores `ts_utc` as a plain `TIMESTAMP` (UTC wall clock) rather than
`TIMESTAMPTZ`, because every `TIMESTAMPTZ` conversion path leads back to ICU.
Per request, the requested zone's UTC-offset spans are generated in Rust and
materialized into a small temp table, and the scope join adds the offset:

```
tz_spans(from_utc, to_utc, offset_secs)   -- ~2 rows per year for Europe/Rome
ts_local = m.ts_utc + to_seconds(z.offset_secs)
```

Spans are found by walking the data range hourly and coalescing runs of equal
offset. Hourly is exact for every IANA transition in the modern era (they land
on whole hours), the walk is bounded by the archive's own date range, and it
costs microseconds — a transition table would be faster and is not worth the
dependency on an API `chrono-tz` does not expose.

Downstream, only *bucketing* uses `ts_local`: day and hour-of-day buckets and
the window predicate. The date-range strings are explicitly UTC, and the
active-session-time gaps are absolute durations, so both keep using `ts_utc` —
shifting them would be wrong, not merely redundant.

**Implementation constraint that follows:** the port MUST NOT emit `LOAD icu`
*or* `AT TIME ZONE`. Both are asserted against in the capability test.

The correctness argument for all of this is the differential gate (D4), which
compares against Postgres — the engine whose timezone handling is the
specification — over real data, including windows that straddle a DST
transition.

## Error handling

| Condition | Behaviour |
|---|---|
| No mirror yet | `503` + `Retry-After`; build runs in background |
| Mirror corrupt | move aside timestamped (never delete), rebuild, log loudly |
| Postgres unreachable at refresh | keep serving; age header climbs; no exit |
| Refresh overruns its interval | single-flight; skip the tick |
| Rows committed out of order | overlap re-scan + idempotent insert picks them up next tick |
| Mirrored columns UPDATEd in Postgres (e.g. `backfill-analytics`) | out of contract — operator runs `hub mirror rebuild`; stats serve from the old mirror until the new file swaps in |
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

**The differential gate runs first** (see D4): old and new implementations over
the same data, exact equality modulo D3's ≤10 boundary groups, before the
Postgres rollups are deleted.

Beyond the two gates: the existing endpoint tests port onto the new path with the
same fixtures and expectations (no second implementation retained); the
incremental-refresh tests assert D2 in all three shapes — a row joining an
existing dedup group, a backfilled row carrying an old timestamp under a new id,
and a **lower-id row becoming visible after higher ids were already mirrored**
(the out-of-order commit case, exercised via the overlap re-scan and asserted to
land exactly once); a rebuild test asserts a `message_id` UPDATE is reflected
after `hub mirror rebuild` with serving uninterrupted; and a cold-start test
asserts `503` then `200`. CI needs no new services — `archive-tests.yml` already
runs Postgres, which is what the mirror reads *from*.

## Risks

| Risk | Handling |
|---|---|
| Silent incompleteness (skipped rows) | overlap re-scan + idempotent inserts; watermark lag on `/v1/healthz/stats` |
| Silent staleness | age headers + `/v1/healthz/stats` + a Gatus check relayed to infra |
| ICU / timezone support | **REAL, and task 1.1 got it backwards** — `AT TIME ZONE` needs `icu` for every zone and `icu` cannot be statically linked from the published crate. Handled by computing offsets in Rust with `chrono-tz` (D7), guarded by `duckdb_capability_test.rs`, and verified against Postgres across a DST transition by the differential gate |
| Binary size / CI build time | **MEASURED, task 1.2**: +40 MB (hub 14 MB → ~54 MB), statically linked with no dylib dependency, ~873 s CPU to build. On a 3-core macos-14 runner that is roughly +5 min, taking the release job from ~5 min to ~10 min. Acceptable; the prebuilt-libduckdb fallback is not needed |
| Sync DuckDB API on an async server | rollups run on `spawn_blocking` with cloned connections; without this every stats request parks a tokio worker for ~0.4 s and nothing in the gates would catch it |
| Disk on m4m | 119 MB today, grows with the archive — headroom check goes in the deploy relay |
| DuckDB resources on a shared box | set explicit `memory_limit` **and** `threads`; m4m also runs the distiller and daemon |
| Port fidelity | differential gate against the outgoing implementation, then the oracle gate |

## Deployment note

After the §2b binary swap, `/v1/stats/*` returns `503 warming` for roughly four
minutes while the first mirror builds (the initial pull from pg1 measured 227 s,
network-bound). **This is expected and is not a rollback trigger** — it belongs in
the relay text, since an unexplained 503 after a swap is exactly the shape that
has caused false alarms before.
