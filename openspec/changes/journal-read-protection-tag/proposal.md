## Why

The client-side review of `distill-thread-origin-filter` (Gitea #84, review 3, finding M2, filed as
#19) found that a protected journal entry can come back to the general model through a door other
than open threads. A protected entry's headline, summary, topics and open questions are written by
the protected Claude model from client material. Any read token gets them from
`GET /v1/journal/entries`, the journal leg of `GET /v1/search` (full-text, semantic and hybrid) and
the open-questions report. The `cchv-find` skill tells agents to read journal hits first. When a lab
session does that, the text lands in its transcript as a tool result, and that day is distilled:

- an entry from a client project carries its client path, which the routing view catches;
- an entry from a **lab** project does not: its path is a lab path and its prose a paraphrase, so the
  group routes general. `distill-route-full-session` creates exactly these lab-project protected
  entries.

The review made parts 1 and 2 below a release gate for `distill-route-full-session`. A second reader,
planner's journal export (#20, owned by planner), needs part 1 to skip protected days.

## What Changes

- **Part 1: every journal read carries the route.** `GET /v1/journal/entries` and every journal hit
  of `GET /v1/search` (all modes) return `route`: `general`, `protected`, or null. It is the route
  as the thread fold counts it (null when unknown or rewritten without it). Open-questions threads
  already carry `origin` (`distill-thread-origin-filter`).
- **Part 2: protected text is tagged.** In those responses, and in the question text of
  protected-origin threads of the open-questions report, every text field of a row whose route is not
  `general` starts with a fixed tag, `[cchv:protected-journal] `. The client's manager adds the tag to
  the routing file as a content term. A transcript that quotes such a field then routes protected
  (class `text_term`), whatever its path. A per-field tag survives a caller that extracts one field
  (`jq '.journal[].summary'`), which a separate flag would not.
- Stored text is unchanged: the tag is added on read only, so the search index, the embeddings and
  the distiller's own use of the rows see the stored text.

## Capabilities

### Modified Capabilities

- `journal-entries`: the browse endpoint returns `route` and tags protected text.
- `archive-search-api`: journal hits return `route` and tag protected text.
- `journal-open-questions`: protected-origin thread wording is tagged in the report.

## Impact

- `crates/hub/src/journal.rs` (`JournalEntry`, `JournalHit`, their queries, one tagging function),
  `crates/hub/src/journal_threads.rs` (report wording).
- `src/services/hubApi.ts` (the optional `route` field in the row types). The webapp shows the tag as
  text; a badge is a later UI change.
- Routing file (client's manager, outside the repo): one new content term, the tag; the floor record
  is re-recorded after that widening edit.
- Not in this change: agent guidance in `cchv-find` (owner: context) to read only
  `route == "general"` rows outside client work (part 3); leaving protected rows out of search by
  default (part 4, optional).
- Ships in the same release as `distill-route-full-session` and `distill-thread-origin-filter`;
  needs migration 0016 from the latter.
