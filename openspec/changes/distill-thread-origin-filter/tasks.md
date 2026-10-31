## 1. Review

- [x] 1.1 Second review by the client-side security reviewer (two separate Claude reviews, merged as M1–M9, 2026-10-31): go, with M1 decided first (no backfill, by the client's manager)
- [x] 1.2 manager-lab's go to implement (2026-10-31 16:23Z), revision 2 per the review

## 2. Hub

- [x] 2.1 Migration 0016: `journal_entries.route` (nullable, check `general`/`protected`) and `route_at`; no backfill; test on a fresh DB that every existing row has no route (M1)
- [x] 2.2 Write path: `EntryPayload.route`, stored with `route_at = generated_at = statement_timestamp()`; other values `400`; a re-post without a route stores null over `general` (M8 f)
- [x] 2.3 Fold: `origin` general only when every mentioning entry has a counted route `general` (route set and `route_at = generated_at`); per-session taint (M7); tests: general-only, one protected mention, one null mention, protected then general wording, general then protected (M8 a), a protected mention older than the window (M8 b), a row rewritten by the pre-0016 upsert (M6), a session shared with a protected day and its unshared twin (M7)
- [x] 2.4 Endpoint: `origin=general` filters before the state filter and the limit (M8 c) and echoes it; any other value `400`; counts mode unchanged
- [x] 2.5 POST check (M5 layer 1): a `general` entry linking or resolving a protected-origin id → `400`, nothing written; `protected` or absent route → stored
- [x] 2.6 Read-back query file (M5 layer 3) and a hub test that plants a violation and expects 1, a clean set 0

## 3. Distiller

- [x] 3.1 `generate` returns the route it ended on; the gate-turned group reports `protected` and keeps the general-only list (M8 e)
- [x] 3.2 `process_group` posts `route`; fetches threads with `origin=general` on the general route, unfiltered on the protected route
- [x] 3.3 Echo checked by value (M8 d); without it no threads; with it, threads whose `origin` is not `general` or missing are dropped with a count warning (M5 layer 2)
- [x] 3.4 Proving test (C5): a project with one protected day, then one general day: the general prompt offers 0 threads from the protected entry; its twin (two general days) offers the first day's threads
- [x] 3.5 End to end on a test hub: post a protected entry with questions, then fetch with `origin=general` → none of them; post a general entry → its questions come back (crates/hub/tests/journal_threads_test.rs: real hub over HTTP, `origin_filter_echoes_…` and `a_general_entry_cannot_link_or_resolve_…`)

## 4. Release

- [ ] 4.1 Ships with `distill-route-full-session` (its task 4.3): hub with 0016, then the daemons, then one distiller reinstall carrying both; on ac's word
- [ ] 4.2 Read back after the first night: entries by route; the read-back query = 0; sent with the route counts to the client's manager
