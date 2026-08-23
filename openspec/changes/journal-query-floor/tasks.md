## 1. Baseline, taken before anything changes

- [x] 1.0 Probe **without a client timeout** and check `pg_stat_activity` for orphaned
      `msg_days` queries before any reading. An abandoned HTTP request leaves the query
      running server-side; six accumulated during this change's own measurement and
      starved each other. Verify: zero active `%msg_days%` queries before each timing run.
- [x] 1.1 Capture `EXPLAIN (ANALYZE, BUFFERS)` for the `healthz_journal` fold query and for
      `journal::pending`, against pg1, in one session. Verify: both plans recorded with node
      type and `shared read`/`hit` block counts, saved into the change folder as
      `baseline.txt` so the "after" has something to be compared with. A timing without
      buffer counts does not count — cache state confounds it.
- [x] 1.2 Record `pg_stat_user_tables` for `messages`: `last_autovacuum`, `last_autoanalyze`,
      `n_dead_tup`, `n_live_tup`. Verify: values captured in the same file. This is the
      visibility-map baseline decision 5 depends on; an index-only scan degrades silently
      without it. **CORRECTED 2026-08-21 (the first version of this line was wrong).**
      `last_autovacuum 2026-08-19` is *not* a backlog: autovacuum is threshold-driven and
      no trigger has been crossed — `n_dead_tup` 929 against a 1,515,886 dead-tuple
      trigger (0.06 %; on an append-mostly archive that trigger will never fire, correctly,
      because there is nothing to reclaim), and `n_ins_since_vacuum` 1,132,642 against a
      1,516,886 insert trigger (75 %). Reading a threshold as a schedule was the error.

      **The real constraint is worse and is structural.** The visibility map is maintained
      by VACUUM, and on this table VACUUM is driven by the *insert* trigger at scale factor
      0.2 — roughly every 1.5 M inserts. Between runs the newest rows carry no VM mark, and
      a 7-day journal window reads exactly that hot range. Measured now: the table is
      **89.5 % VM-marked overall** (501,384 of 559,953 pages), but **46.4 % of the window's
      rows** (1,132,642 of 2,439,743) sit on pages inserted since the last vacuum. So an
      index-only scan incurs `Heap Fetches` on ~46 % of what it touches **today**, and that
      fraction **oscillates with the vacuum cycle** — near 0 % just after a vacuum, ~60 %+
      just before the next fires.
- [x] 1.3 **Capture 1.2 BEFORE any `pg_stat_reset()`.** A clean per-day temp rate wants the
      counters reset (`stats_reset` on `cchv_archive` is NULL, so the 3,781 GB / 295,895
      temp files are lifetime totals and no rate can be derived from them — infra's point).
      But `pg_stat_reset()` is database-wide and clears `pg_stat_user_tables` too, which is
      exactly where 1.2's `last_autovacuum` lives. Verify: 1.2's values are written down
      first; only then reset, and only if the four-arm plan actually needs a rate rather
      than a per-request delta. Per-request `pg_stat_database` deltas around one call —
      what infra used — need no reset at all and are the cheaper instrument.

## 2. The index

> **DECISION 2026-08-23 (measured, phase 3 run early): DO NOT BUILD IT.** Task 3.4's
> `work_mem` arms were run *before* 2.2's relay, because 2.2 spends infra's production
> window and the free lever had not been measured. `work_mem = 64MB` takes
> `healthz_journal` from 3,545 ms to 2,252 ms (**1.57x**) and its temp writes from 17,163
> blocks to **zero**, at no storage cost, no DDL, and no coordination. Against that
> baseline the covering index is worth **~124 ms, about 5 %** of the endpoint — measured
> via the forward-`pending` index-only fold (1,493 ms) against the 64 MB fold (1,617 ms) —
> for ~400 MB, a production `CREATE INDEX CONCURRENTLY` on 8.1 M rows, and permanent write
> amplification on the hottest table. The 5 % is also *generous* to the index: the proxy
> elides `max(created_at)`, and everything was measured at the optimistic end of the vacuum
> cycle, where an index-only scan looks its best. Evidence: `workmem-arms.txt`,
> `baseline.txt`, #36 comments 7505 and 7511.
>
> Consequences: **2.2 is not sent** — not blocked, not deferred pending information, but
> measurably not worth doing. 2.1's operator step stays in `deployment.md` marked *not
> built*, so a future reader can build it if the table's shape changes. 3.1 / 3.2 / 3.3 /
> 3.6 are moot while the index does not exist, and are marked so rather than deleted, so
> the record shows what was planned and why it was not run.

- [x] 2.1 Add the index build to `docs/archive/deployment.md` §1 as an operator step —
      **not** a `migrations/*.sql`, since `CREATE INDEX CONCURRENTLY` cannot run in a
      transaction and `sqlx` migrations do. Include the `DROP INDEX CONCURRENTLY` recovery
      for the INVALID-index failure mode, and the ~300 MB / 209 GB-free sizing. Verify:
      the section names the exact statement, the recovery, and when to run it.
- [~] 2.2 **NOT SENT — see the decision above.** Relay the build to infra (`cchv-deploy`, Channel 0 while `ac/infra#98` is open):
      off-peak, `CREATE INDEX CONCURRENTLY messages_journal_fold_idx ON messages
      ("timestamp", session_id) INCLUDE (created_at)`. Carry the expected duration, the
      storage delta, the recovery command, and the fact that **nothing breaks if it is not
      built** — no code references it. Verify: ack received; index reported `indisvalid`.

      **AMENDED 2026-08-23 after phase 1 — the relay must carry `~1.8-2x`, never `~14x`.**
      `baseline.txt` contains a measured control the design did not have: `journal::pending`
      in forward shape already runs an **index-only scan over the same window, the same
      2,482,615 rows and the same 100 groups**, and its fold takes **1,493 ms against
      `healthz_journal`'s seq-scan fold at 3,158 ms — 2.11x**, with temp writes 172.4 MB ->
      52.1 MB. Carried through, the endpoint lands near **2,150 ms against 3,818 ms (~1.8x)**
      and is Amdahl-capped at 4.5x even with a free fold. State also that the index is
      expected to change **`healthz_journal` only**: both `pending` shapes are already
      index-only on the existing `(session_id, "timestamp")` index, and the proposed index
      leads on `"timestamp"`, which backfill-shape `pending` has no range predicate on.

## 3. Prove it, or say it did not work

- [x] 3.0 **Record where in the vacuum cycle each measurement was taken** — `n_ins_since_vacuum`
      and `relallvisible`/`relpages` immediately before and after every timing run. Because
      the unmarked fraction of the window swings from ~0 % to ~60 % across an insert cycle,
      the *same index* measures anywhere between "nearly free" and "half-wasted" depending
      on when you look. A single reading without its cycle position is not a measurement of
      the index; it is a measurement of the day. Verify: every arm's readings carry both
      numbers.

      **And know which way the current position biases the result before believing it.**
      The cycle position on 2026-08-21 was **75 %** (`n_ins_since_vacuum` 1,135,443 of
      1,516,440), i.e. near the *pessimistic* end: a run taken then maximises `Heap
      Fetches` and **understates** the index. That is the dangerous direction, because it
      is the one that gets a correct change rejected — a flattering reading invites a
      second look, an unflattering one usually does not. So either take each arm **twice,
      at both ends of the cycle** (just after an insert-triggered VACUUM and just before
      the next), or take it once and state explicitly which end it came from and which way
      it biases. A result quoted without that is not interpretable.
- [~] 3.1 **MOOT (moot: no index to re-measure against)** Re-run 1.1's two `EXPLAIN (ANALYZE, BUFFERS)` and diff against `baseline.txt`.
      Verify: node is `Index Only Scan using messages_journal_fold_idx`; `Heap Fetches` is
      small relative to rows returned; `shared read+hit` drops by roughly an order of
      magnitude (baseline ≈ 170 k pages for the window).

      **AMENDED 2026-08-23 — assert this for `healthz_journal` ONLY.** Both `journal::pending`
      shapes already scan index-only via `messages_session_timestamp_idx` at baseline, so
      "it is an index-only scan" is **not a check** there — it passes before the change.
      For `pending` the only meaningful assertion is that plan and timing are **unchanged**,
      i.e. the new index was not chosen and did not make things worse.
- [~] 3.2 **MOOT (moot: no plan change to check for a behaviour change)** Confirm the result set is **unchanged** — same groups, same count, same
      `latest_arrival` values as the baseline run. Verify: a diff of the two result sets is
      empty. This is the check that catches a plan change that quietly became a behaviour
      change; it is the whole reason the index is not partial.
- [~] 3.3 **MOOT (moot: the planner was never offered the index)** If the planner declines the index: keep it, record the plan it chose, and measure
      decision 1's fallback (`SET LOCAL random_page_cost` on just these two statements)
      before deciding. Verify: whichever way it goes, the reading is on #36 — a slow
      endpoint left slow silently is the outcome this task exists to prevent.
- [x] 3.4 Measure `SET LOCAL work_mem` **separately from the index**, so neither is
      credited with the other's win: re-run the two queries with work_mem raised, with and
      without the index. Verify: four readings recorded — baseline, index only, work_mem
      only, both — and `temp read/written` drops to ~0 in the work_mem arms. Complementary,
      not alternative: the index removes bytes read, work_mem removes bytes written.

      **AMENDED 2026-08-23 — that split is too clean, so DO NOT ADD THE TWO ARMS.** Phase 1
      measured the index-only path removing **70 % of the temp writes on its own**
      (172.4 MB -> 52.1 MB) with no memory limit raised: it replaces an `external merge`
      sort with an `Incremental Sort` over presorted input. The two levers therefore
      **overlap on bytes written**, and index-only + work_mem-only summed would double-count
      it. That makes the four-arm design necessary rather than tidy — the `both` arm is the
      only one that answers what they are worth together.
      **Sizing, measured on prod by infra 2026-08-21:** one request spills **88.4 MB across
      3 temp files** (≈22× the 4 MB `work_mem`, one file per parallel process) and takes
      4.04 s; at Gatus's 300 s cadence that alone is **~24.9 GB/day of temp writes**. Our
      own `EXPLAIN` reading was 16,701 blocks ≈ 136.8 MB. **The gap is unexplained and must
      not be papered over:** both readings were taken with *zero* pending groups (our plan's
      top-level Sort reports `rows=0.00`), so it is not "floor versus with-work". Candidates
      are `EXPLAIN ANALYZE` instrumentation and an hour of window drift between the two.
      Re-measure both ways in the same session before quoting either number.
- [x] 3.5 **Decide whether to ask ac for `autovacuum_vacuum_insert_scale_factor = 0.02`
      on `messages`** (~150 k inserts between vacuums instead of ~1.5 M, keeping the window
      mostly VM-marked). infra has named it and deliberately **not run it** — it is a prod
      DDL on their box and therefore ac's call. Only worth asking once 3.0–3.1 show the
      index's win is genuinely capped by `Heap Fetches` rather than by the planner ignoring
      the index. Verify: either a measured case put to ac with numbers, or a recorded
      decision that the uncapped win is already sufficient.
- [~] 3.6 **MOOT (moot: no new index to amplify writes)** Watch one heavy ingest after the build for write amplification on `messages`.
      Verify: ingest duration compared against a pre-build batch of comparable size; noted
      even if unchanged, so a later regression has a baseline.

## 4. The row-count caveat (the #41 half that is documentation)

- [x] 4.1 State in `docs/archive/deployment.md` and `AGENTS.md` (where the archive's size is
      quoted) that **a row in `messages` is not a conversation turn** — 91.1 % of
      `claude`-provider rows are sidecar state records with `content IS NULL`, so every raw
      count overstates conversation by ~11×, and it is why ~half of any backfill posts
      `skip`. Verify: `grep -n "conversation turn" AGENTS.md docs/archive/deployment.md`
      hits both; existing figures in those files that quote raw counts carry the caveat.
- [x] 4.2 Leave the analytics-side conversation-only count to **#41**, with a comment
      recording why it is not here: it is an API change with its own surface and its own
      decision about what a "message" means to a reader. Verify: comment on #41.

## 5. Close out

- [ ] 5.1 Close #36 with the before/after plans and buffer counts (not "fixed in vX.Y.Z").
      Verify: the issue carries both `EXPLAIN` extracts.
- [ ] 5.2 Record the outcome in `docs/archive/deployment.md` — including, if it happened,
      the planner declining the index, which is the more useful record of the two.
- [ ] 5.3 Archive this change (`openspec archive`), syncing both deltas into the main specs.
      Verify: `openspec validate --specs` passes and `journal-health` carries the
      does-not-read-payloads requirement.
