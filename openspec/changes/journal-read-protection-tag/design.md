## Context

`distill-thread-origin-filter` adds `journal_entries.route` and `route_at`; a route counts only when
`route_at = generated_at`. Journal text leaves the hub through `browse` (`JournalEntry`),
`search_journal` (`JournalHit`, used by the full-text, semantic and hybrid modes of the journal leg)
and the open-questions report (`Thread.question`). The distiller routes a group protected when a
content term appears anywhere in the full text of its sessions (`distill-route-full-session`).

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
5. **Snippets.** A search hit's highlighted snippet (if present) is tagged like the other fields.

## Alternatives considered

- A separate `protected: true` field only: lost when a caller extracts one field (`jq`), and the
  distiller would have to parse JSON structure in transcripts. Kept as part 1 (`route`) next to the
  tag, not instead of it.
- Leave protected rows out of search and browse by default: changes what every reader sees; kept as
  an option (part 4) for after the tag is measured.
- Tag at write time: would change the search index, the embeddings and the thread wording the
  distiller reuses.

## Risks

- **More protected days for lab sessions that read the journal.** That is the point; the class
  counts (`text_term`) show the volume after one night.
- **Human readers see the tag.** Acceptable; a UI badge can replace it later.
- **A caller that strips the tag** before passing text on defeats part 2; part 1's `route` still
  tells such a caller what it holds, and part 3's guidance asks it not to.
