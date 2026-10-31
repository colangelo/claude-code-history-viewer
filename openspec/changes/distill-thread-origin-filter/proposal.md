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
- **Existing entries.** The migration marks as `general` the existing entry rows written by a GPT
  model (`model LIKE 'gpt-%'`): their wording was already produced by the general backend, so
  offering it to that backend again exposes nothing new. Every other existing row stays null, which
  is protected origin.

## Capabilities

### Modified Capabilities

- `journal-entries`: entries store the route they were generated on; the distiller offers only
  general-origin threads to a general call.
- `journal-open-questions`: threads carry their origin; the report filters by origin on request.

## Impact

- `migrations/0016_journal_entry_route.sql` (column, check constraint, backfill of GPT rows).
- `crates/hub/src/journal.rs` (payload and insert), `crates/hub/src/journal_threads.rs` (fold, param,
  echo).
- `scripts/cchv-distill.py` (`generate` reports the route it used; `process_group` posts it and
  fetches threads per route), tests.
- Release order: hub (with the migration) before the distiller. A new distiller against an old hub
  offers no threads on general days (fail closed); an old distiller against a new hub posts no route,
  so its entries are protected origin.
- Session-sticky protection (M6, "one mechanism" with this filter in the review) already holds
  through `distill-route-full-session`: the route is decided on every session's whole history.
