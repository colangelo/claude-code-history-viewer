## Context

Threads (`journal-open-questions`) are folded on the hub from `journal_entries.open_questions` and
`open_question_threads`; the latest wording wins. The distiller fetches
`/v1/journal/open-questions?project=…&before=<entry date>&days=30&state=open,quiet&limit=30` and
lists the threads in the prompt. Since `distill-route-full-session`, the route is decided by
`route_group` before generation, and `generate` can still turn a group protected at the send-time
gate (class `prompt`). Today nothing on an entry row says which route wrote it; `model` is the
serving model, and a general call on the `claude` backend also serves a Claude id, so the model
alone cannot tell the routes apart.

## Decisions

1. **Store the route, not infer it.** `journal_entries.route TEXT NULL CHECK (route IN
   ('general', 'protected'))`. `EntryPayload.route: Option<String>`; any other value is a `400`.
   The write path stores it as given; a re-distilled day replaces the row, route included.
2. **The route posted is the route used.** `generate` returns the route class it ended on (`None`
   for general), so a group that `route_group` found general but the gate turned protected posts
   `protected`. A skip made without any call posts no route (null): it has no questions.
3. **Origin is per thread, from every mention.** In the fold, a thread is general origin when every
   entry mentioning it has route `general`. One protected or null mention makes it protected origin
   for good, even if the latest wording came from a general entry: an earlier protected wording may
   have shaped the later one. Resolutions carry no text and do not count.
4. **The filter is on the hub, and the distiller checks it was applied.** `origin=general` (the only
   accepted value; anything else is a `400`) filters the project report and echoes
   `"origin": "general"`. Axum ignores unknown query parameters, so an old hub would silently return
   every thread: the distiller therefore offers threads to a general call only when the echo is
   present, and otherwise logs a warning and offers none.
5. **Fetch after routing, per route.** `process_group` fetches threads after `route_group`: with
   `origin=general` when it returned general, without the filter when protected. If the gate later
   turns the group protected, it keeps the general-only list (fewer threads, never more).
6. **Backfill only what the general backend already wrote.** The migration sets `route = 'general'`
   where `model LIKE 'gpt-%'` (rows the general backend generated; the protected route has only ever
   served Claude ids). All other rows stay null, which is protected origin. Without the backfill,
   general days would lose thread continuity for up to 30 days; with it, nothing the general backend
   did not already produce is offered to it. Census on pg1, 2026-10-31 (entry rows): `gpt-5.6-sol`
   799 and `gpt-6.1-sol` 92 become `general`; `claude-haiku-5-5` 134 (the protected route since
   10-20) and `haiku` 37 (July, the old `claude` backend, outside any 30-day window) stay null.
7. **Counts mode unchanged.** The per-project counts (no `project`) ignore `origin`.

## Alternatives considered

- Filter in the distiller from a per-thread field only: rejected as the only check, because an old
  hub returns no field and the distiller would have to treat that as protected anyway; the echo
  does the same with one flag. The thread still carries `origin` for the report and tests.
- Infer the route from `model`: rejected, a general `claude` backend serves Claude ids too.
- Drop threads from general prompts entirely: loses the feature for every lab project to fix a case
  the origin rule handles.

## Risks

- **Rollout window:** until the hub is swapped, a new distiller offers no threads on general days.
  Order: hub first.
- **Re-distilled days:** a dirty protected day re-distilled as general (after a routing-file
  narrowing) replaces the row with `general`; its old threads keep their protected mentions only if
  another protected entry still mentions them. A narrowing already needs the client's manager's
  word (the floor record).
