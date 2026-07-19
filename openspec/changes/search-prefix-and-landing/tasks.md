# Tasks: search-prefix-and-landing

- [x] 1.1 `fts::prefix_tsquery` + unit tests (plain → `'tok':*` AND-chain;
  websearch syntax → None; simple-config tokenization parity).
- [x] 1.2 CTE-combined tsquery in `search.rs::message_hits` and
  `journal.rs::search_journal`; `.sqlx` regenerated.
- [x] 1.3 `position` on `SearchHit` via NULLS-LAST-exact counting subquery;
  integration test asserts it indexes into the browse listing.
- [x] 1.4 Pool: `min_connections(2)` + `acquire_timeout(5s)` (#17).
- [x] 2.1 Client windowed messages: land on the hit's page, highlight +
  center the match, "Load earlier", range progress; old-hub fallback.
- [x] 2.2 Riders: hit timestamps, `/` shortcut, tablist arrows, tokenless
  connect (form + storage + test).
- [x] 3.1 i18n ×5 + types; `i18n:validate`.
- [x] 3.2 Gate: tsc, 920 vitest, lint, cargo fmt/clippy/test (56 hub tests).
- [x] 3.3 E2E: local v0.10.1 hub + seeded 450-message session — prefix hits
  (journal + message), landing at 401–450/450 with centered highlight,
  load-earlier to start, `/` and arrow keys, zero console errors.
