## Context

`distill-thread-origin-filter` adds `journal_entries.route` and `route_at`; a route counts only when
`route_at = generated_at`. Journal text leaves the hub through `browse` (`JournalEntry`),
`search_journal` (`JournalHit`, used by the full-text, semantic and hybrid modes of the journal leg)
and the open-questions report (`Thread.question`). The distiller routes a group protected when a
content term appears anywhere in the full text of its sessions (`distill-route-full-session`).

**Revision 2 (2026-10-31)** follows the client-side re-test of revision 1 (one Sonnet review,
"closes with conditions" C1–C8): the message-search snippet (C1), the whole-session dependency stated
and tested (C3), hub-produced fixtures and the field checks (C4, C6), and the `route=general` filter
(C7 step 1). C2 (a count of the pre-release window), C5 and C7 step 2 (ac's decision) and C8 (guidance,
context) are release steps outside the code.

## Decisions

1. **One counted route, one place.** Every read query selects
   `CASE WHEN route_at = generated_at THEN route END AS route`, the same rule as the thread fold.
2. **Tag on read, in one function.** `protect_text(route, &mut fields)` prefixes headline, summary,
   each topic and each open question with `[cchv:protected-journal] ` when the route is not
   `general`. It runs on every `JournalEntry` and `JournalHit` just before serialization, and on the
   question text of every protected-origin thread in the report. Stored rows, `search_text` and
   embeddings keep the untagged text, so ranking and the distiller are unchanged.
3. **The tag is a content term, owned outside the repo.** The tag itself names no client and may sit
   in the repo; adding it to the routing file is the client's manager's edit, followed by
   `--record-routing-floor`. Until that edit lands, the tag only marks the text for human readers
   and for consumers like planner's export.
4. **Null is tagged.** Rows written before 0016, or by an older distiller, have no route and are
   tagged: they may hold client paraphrase (review 3, M1).
5. **Snippets (re-test F1, F4).** The journal leg has no snippet field. The message leg does: a
   message whose stored `search_text` holds the tag (a transcript that quoted a protected read) gets
   a snippet that starts with the tag, whatever the match highlighted or cut. A second, runtime query
   over the page's message ids finds them; the compile-time-checked message query is unchanged.
6. **Every field, by construction (F4).** `JournalEntry::protected` and `JournalHit::protected`
   destructure their row exhaustively, so a new field fails to compile until it is placed; a unit test
   walks the serialized strings and pins the set of string fields.
7. **Once, and not read back (F5).** `tag_text` does nothing to a field that already starts with the
   tag; the distiller strips a leading tag from offered thread wording, so the model does not copy it
   into new stored text.
8. **`route=general` on every journal read (C7 step 1).** Browse and the journal leg of search accept
   `route=general` (any other value `400`) and return only rows whose counted route is `general`, in
   every mode; the report already has `origin=general`. Additive: no default changes.
9. **One tag, from the hub (C4).** `PROTECTION_TAG` lives in `crates/hub/src/journal.rs`. A hub test
   writes `scripts/fixtures/journal-read-shapes.json` from the serializer (drift fails); the distiller
   carries a copy for stripping, and a test asserts it equals the fixture's tag and that the routing
   term matches it. `scripts/journal-tag-probe.py` routes the fixture's shapes through a distiller and
   the LIVE routing file: the repeatable release probe.

## Alternatives considered

- A separate `protected: true` field only: lost when a caller extracts one field (`jq`), and the
  distiller would have to parse JSON structure in transcripts. Kept as part 1 (`route`) next to the
  tag, not instead of it.
- Leave protected rows out of search and browse by default: changes what every reader sees; kept as
  an option (part 4) for after the tag is measured.
- Tag at write time: would change the search index, the embeddings and the thread wording the
  distiller reuses.

## Risks

- **Paraphrase depends on #10 (re-test F2, a precondition).** A paraphrase without a quote is caught
  only because `route_group` reads every session of the group whole (`distill-route-full-session`):
  the tag anywhere in the session protects the whole group, on every day of that session. This change
  ships with `distill-route-full-session` or the paraphrase door stays open; tests pin the three
  cases (same session, another session of the group, a later day of the session).
- **A paraphrase that leaves the group (re-test F3).** A peer message, issue comment, commit or file
  written from a protected read lands in another group without the tag. Nothing here catches it; ac
  decides between a dated residual and the default-exclude step (C7 step 2), as one release-row item.
- **Shapes that cut the tag.** A hard wrap inside the tag, or a view that starts after it, loses it;
  tests document both as general on the tag alone. Message-search snippets are prefixed (C1).
- **The pre-release window (C2).** Transcripts that quoted journal reads before the release hold
  untagged copies. A count-only scan at release (lab tool results holding a journal-read shape) goes
  to the client's manager, with the still-pending groups pinned protected or re-pended.
- **cchv's own sessions route protected** while they work on this change: the repo holds the literal
  tag (constant, fixture, tests). That volume is not leakage. Guidance files must not quote the
  literal tag (re-test F6): every session that loads one would route protected.

- **More protected days for lab sessions that read the journal.** That is the point; the class
  counts (`text_term`) show the volume after one night.
- **Human readers see the tag.** Acceptable; a UI badge can replace it later.
- **A caller that strips the tag** before passing text on defeats part 2; part 1's `route` still
  tells such a caller what it holds, and part 3's guidance asks it not to.
