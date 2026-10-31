## 1. Review

- [ ] 1.1 Second review by the client-side security reviewer (the same two separate Claude reviews as #10), via the client's manager
- [ ] 1.2 manager-lab's go to implement, after 1.1

## 2. Hub

- [ ] 2.1 Migration 0016: `journal_entries.route` (nullable, check `general`/`protected`), backfill `general` where `model LIKE 'gpt-%'`; test on a fresh DB that a GPT row is `general`, a Claude row and a skip stay null
- [ ] 2.2 Write path: `EntryPayload.route`, stored as given, other values `400`; tests for each value, absent and invalid
- [ ] 2.3 Fold: a thread's `origin` is `general` only when every mentioning entry is `general`; tests: general-only, one protected mention, one null mention, protected first then general wording
- [ ] 2.4 Endpoint: `origin=general` filters the project report and echoes it; any other value `400`; counts mode unchanged

## 3. Distiller

- [ ] 3.1 `generate` returns the route it ended on; the gate-turned group reports `protected`
- [ ] 3.2 `process_group` posts `route`; fetches threads with `origin=general` on the general route, unfiltered on the protected route
- [ ] 3.3 No echo from the hub → no threads offered to a general call, with a warning
- [ ] 3.4 Proving test (C5): a project with one protected day, then one general day: the general prompt offers 0 threads from the protected entry; its twin (two general days) offers the first day's threads
- [ ] 3.5 End to end on a test hub: post a protected entry with questions, then fetch with `origin=general` → none of them; post a general entry → its questions come back

## 4. Release

- [ ] 4.1 Ships with or before `distill-route-full-session` (its task 4.3); hub (migration) before the distiller; on ac's word
- [ ] 4.2 Read back: count entries by route after the first night; a general prompt's offered threads all general origin (log line `threads … offered N` against the report)
