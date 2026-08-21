## 1. Baseline, taken before anything changes

- [ ] 1.1 Capture `EXPLAIN (ANALYZE, BUFFERS)` for the `healthz_journal` fold query and for
      `journal::pending`, against pg1, in one session. Verify: both plans recorded with node
      type and `shared read`/`hit` block counts, saved into the change folder as
      `baseline.txt` so the "after" has something to be compared with. A timing without
      buffer counts does not count — cache state confounds it.
- [ ] 1.2 Record `pg_stat_user_tables` for `messages`: `last_autovacuum`, `last_autoanalyze`,
      `n_dead_tup`, `n_live_tup`. Verify: values captured in the same file. This is the
      visibility-map baseline decision 5 depends on; an index-only scan degrades silently
      without it.

## 2. The index

- [ ] 2.1 Add the index build to `docs/archive/deployment.md` §1 as an operator step —
      **not** a `migrations/*.sql`, since `CREATE INDEX CONCURRENTLY` cannot run in a
      transaction and `sqlx` migrations do. Include the `DROP INDEX CONCURRENTLY` recovery
      for the INVALID-index failure mode, and the ~300 MB / 209 GB-free sizing. Verify:
      the section names the exact statement, the recovery, and when to run it.
- [ ] 2.2 Relay the build to infra (`cchv-deploy`, Channel 0 while `ac/infra#98` is open):
      off-peak, `CREATE INDEX CONCURRENTLY messages_journal_fold_idx ON messages
      ("timestamp", session_id) INCLUDE (created_at)`. Carry the expected duration, the
      storage delta, the recovery command, and the fact that **nothing breaks if it is not
      built** — no code references it. Verify: ack received; index reported `indisvalid`.

## 3. Prove it, or say it did not work

- [ ] 3.1 Re-run 1.1's two `EXPLAIN (ANALYZE, BUFFERS)` and diff against `baseline.txt`.
      Verify: node is `Index Only Scan using messages_journal_fold_idx`; `Heap Fetches` is
      small relative to rows returned; `shared read+hit` drops by roughly an order of
      magnitude (baseline ≈ 170 k pages for the window).
- [ ] 3.2 Confirm the result set is **unchanged** — same groups, same count, same
      `latest_arrival` values as the baseline run. Verify: a diff of the two result sets is
      empty. This is the check that catches a plan change that quietly became a behaviour
      change; it is the whole reason the index is not partial.
- [ ] 3.3 If the planner declines the index: keep it, record the plan it chose, and measure
      decision 1's fallback (`SET LOCAL random_page_cost` on just these two statements)
      before deciding. Verify: whichever way it goes, the reading is on #36 — a slow
      endpoint left slow silently is the outcome this task exists to prevent.
- [ ] 3.4 Watch one heavy ingest after the build for write amplification on `messages`.
      Verify: ingest duration compared against a pre-build batch of comparable size; noted
      even if unchanged, so a later regression has a baseline.

## 4. The row-count caveat (the #41 half that is documentation)

- [ ] 4.1 State in `docs/archive/deployment.md` and `AGENTS.md` (where the archive's size is
      quoted) that **a row in `messages` is not a conversation turn** — 91.1 % of
      `claude`-provider rows are sidecar state records with `content IS NULL`, so every raw
      count overstates conversation by ~11×, and it is why ~half of any backfill posts
      `skip`. Verify: `grep -n "conversation turn" AGENTS.md docs/archive/deployment.md`
      hits both; existing figures in those files that quote raw counts carry the caveat.
- [ ] 4.2 Leave the analytics-side conversation-only count to **#41**, with a comment
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
