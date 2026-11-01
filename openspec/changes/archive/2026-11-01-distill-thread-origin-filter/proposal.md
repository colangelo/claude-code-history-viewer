## Why

The client-side review of the distiller (Gitea #84, finding #12, P2; again as M7 in the review of
`distill-route-full-session` and as re-test condition C5) found that open threads leak across
routes. Before writing an entry, the distiller offers the project's unresolved open questions from
the last 30 days. A protected day's entry is written by the protected Claude model from client
material, and its open questions are worded from that material. On a later general day of the same
project, those questions go into the prompt sent to the general backend. The send-time gate catches
a question that names a marker or term, but not a paraphrase.

`distill-route-full-session` makes this more frequent: it protects groups in lab projects whose
sessions touched client folders, and each such day leaves up to five client-derived questions in a
lab project, offered to that project's later general days. The review made this filter a release
gate for that change: it ships with it or before.

## What Changes

- **Entries record their route.** `journal_entries` gains a nullable `route` (`general` or
  `protected`). The distiller posts the route the entry was actually generated on (a group that the
  send-time gate turned protected posts `protected`). A post without it (an older distiller) stores
  null.
- **Threads carry their origin; the report filters on it.** A thread is of general origin only
  when every entry that mentions it has route `general`; a mention by a protected entry or by an
  entry with no route makes it protected origin. `GET /v1/journal/open-questions` gains
  `origin=general`, which returns only general-origin threads and echoes `"origin": "general"` in the
  response.
- **The distiller asks for general threads on the general route.** A group routed general fetches
  threads with `origin=general` and offers none if the hub does not echo the filter (a hub without
  this change). A protected group still gets every thread.
- **The invariant is held in three places.** The hub refuses a `general` entry that links or
  resolves a protected-origin thread; the distiller drops any offered thread whose own `origin` is
  not `general`; an independent SQL read-back must count 0 such links.
- **Per session, and safe across a hub rollback.** A thread is also protected origin when an entry
  mentioning it shares a session with a protected day of the project. A route counts only when it was
  written together with the row, so a row rewritten by an older hub falls back to protected.
- **No backfill.** Every existing entry keeps no route, which is protected origin (decided by the
  client's manager in review: rows written under the routing #10 fixes include paraphrases of client
  material). General days lose thread links for at most 30 days, once.

**Revision 2** follows the client-side review (M1–M9, go): M1 no backfill; M5–M8 as above. M2 and
M3 (protected entries reaching other readers) are separate follow-ups, M4 is a fix to
`distill-route-full-session`, M9 joins the distiller's error-log issue.

## Capabilities

### Modified Capabilities

- `journal-entries`: entries store the route they were generated on; the distiller offers only
  general-origin threads to a general call.
- `journal-open-questions`: threads carry their origin; the report filters by origin on request.

## Impact

- `migrations/0016_journal_entry_route.sql` (`route`, `route_at`, check constraint; no backfill).
- `scripts/sql/journal-thread-origin-readback.sql` (the read-back).
- `crates/hub/src/journal.rs` (payload, upsert, the POST origin check), `crates/hub/src/journal_threads.rs`
  (fold with origin and session taint, param, echo).
- `scripts/cchv-distill.py` (`generate` reports the route it used; `process_group` posts it and
  fetches threads per route), tests.
- Release order: hub (with the migration) before the distiller. A new distiller against an old hub
  offers no threads on general days (fail closed); an old distiller against a new hub posts no route,
  so its entries are protected origin.
- Session-sticky protection: `distill-route-full-session` decides each group on its sessions' whole
  history; the per-session origin rule above covers days distilled before a later protected day.
