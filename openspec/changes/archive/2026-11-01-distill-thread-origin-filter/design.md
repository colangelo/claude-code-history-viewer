## Context

Threads (`journal-open-questions`) are folded on the hub from `journal_entries.open_questions` and
`open_question_threads`; the latest wording wins. The distiller fetches
`/v1/journal/open-questions?project=…&before=<entry date>&days=30&state=open,quiet&limit=30` and
lists the threads in the prompt. Since `distill-route-full-session`, the route is decided by
`route_group` before generation, and `generate` can still turn a group protected at the send-time
gate (class `prompt`). Today nothing on an entry row says which route wrote it; `model` is the
serving model, and a general call on the `claude` backend also serves a Claude id, so the model
alone cannot tell the routes apart.

**Revision 2 (2026-10-31)** follows the client-side review of revision 1 (two separate Claude
reviews, merged as M1–M9, verdict "go, with M1 decided first"): no backfill (M1, decided by the
client's manager), the origin invariant held in three places (M5), a stale route after a hub
rollback (M6), a per-session rule (M7) and six more tests (M8). M2 (protected entries returning
through lab transcripts) and M3 (an export into another repo) are separate follow-ups; M4 is a fix
to `distill-route-full-session`; M9 joins the error-log issue.

## Decisions

1. **Store the route, not infer it.** `journal_entries.route TEXT NULL CHECK (route IN
   ('general', 'protected'))` and `route_at TIMESTAMPTZ NULL`. `EntryPayload.route:
   Option<String>`; any other value is a `400`. The write path stores it as given; a re-distilled day
   replaces the row, route included, and a re-post without a route stores null over an earlier one.
2. **A route counts only when it was written with the row (M6).** The upsert sets `generated_at`
   and `route_at` from one `statement_timestamp()`, on insert and on update. An older hub binary
   (after a rollback) rewrites `generated_at` but not `route`/`route_at`, so the two differ, and the
   fold treats such a row as having no route.
3. **The route posted is the route used.** `generate` returns the route class it ended on (`None`
   for general), so a group that `route_group` found general but the gate turned protected posts
   `protected`. A skip made without any call posts no route: it has no questions.
4. **Origin is per thread, from every mention.** In the fold, a thread is general origin when every
   entry mentioning it has a counted route `general`. One protected or null mention makes it
   protected origin for good, whichever entry holds the latest wording. Resolutions carry no text and
   do not count. The fold reads every row of the project before `before`, not just the window, so an
   old protected mention still counts.
5. **Per session (M7).** A thread is also protected origin when any entry mentioning it shares a
   session id with a protected or no-route `entry` row, or with a `protected` skip row, of the same
   project. A day distilled general before a later day of the same session routed protected thus
   stops offering its threads. Skips without a route (no call) do not taint.
6. **The filter is on the hub, and the distiller checks it twice (M5 layers 1–2).**
   `origin=general` (the only accepted value; anything else is a `400`) filters the project report
   before the state filter and the limit, and echoes `"origin": "general"`. Axum ignores unknown query
   parameters, so an old hub would silently return every thread: the distiller offers threads to a
   general call only when the echo equals `general`, and then drops every thread whose own `origin`
   is not `general` (or is missing), logging the count.
7. **The hub refuses a general entry that links a protected thread (M5 layer 1).** On POST with
   route `general`, the hub folds the project's other rows and rejects with `400` (ids and counts
   only in the message) an entry whose `open_question_threads` or `resolved_threads` name a thread
   that is not general origin.
8. **An independent read-back (M5 layer 3).** `scripts/sql/journal-thread-origin-readback.sql`
   counts `general` rows that link or resolve an id with an earlier mention in a row whose counted
   route is not `general`. It must return 0; it runs on pg1 at the read-back and is sent to the
   client's manager with the route counts. A hub test plants a violation and expects 1.
9. **Fetch after routing, per route.** `process_group` fetches threads after `route_group`: with
   `origin=general` when it returned general, without the filter when protected. If the gate later
   turns the group protected, it keeps the general-only list (fewer threads, never more) and posts
   `protected`.
10. **No backfill (M1).** Every existing row keeps a null route, which is protected origin. Rows
    written by GPT under the routing that `distill-route-full-session` fixes include paraphrases of
    client material, and re-offering them would be a new send every night, kept alive by
    continuation. The cost is a one-time loss of thread links on general days for at most 30 days;
    no content is lost. A later backfill (for example, `general` only where the released routing
    still finds the group general, counts to the client's manager first) needs ac's word.
11. **Counts mode unchanged.** The per-project counts (no `project`) ignore `origin`.

## Alternatives considered

- Filter in the distiller from a per-thread field only: rejected as the only check, because an old
  hub returns no field; the echo and the per-thread check together cover both.
- Infer the route from `model`: rejected, a general `claude` backend serves Claude ids too.
- Drop threads from general prompts entirely: loses the feature for every lab project to fix a case
  the origin rule handles.
- Taint per project (any later protected day): rejected by both reviewers; a later day shapes a
  thread only by mentioning it, and it would amount to no threads on general days.
- GPT-only backfill (revision 1): rejected in review (M1).

## Risks

- **Rollout window:** until the hub is swapped, a new distiller offers no threads on general days.
  Order: hub with 0016, daemons, then the distiller.
- **Re-distilled days:** a dirty protected day re-distilled as general (after a routing-file
  narrowing) replaces the row with `general`; its threads stay protected while any other protected or
  same-session row mentions them. A narrowing already needs the client's manager's word.
- **Same trust as other fields:** `route` is declared by whoever posts the entry, like every other
  entry field (any machine token allowed to post entries).
