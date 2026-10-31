## 1. Review

- [x] 1.1 Client-side re-test of revision 1 (one Sonnet review, 2026-10-31): closes with conditions C1–C8
- [x] 1.2 manager-lab's go to implement (2026-10-31 16:55Z), revision 2 per the re-test

## 2. Hub

- [x] 2.1 `route` (counted) on `JournalEntry` and `JournalHit`, all search modes; tests for general, protected, null, a stale `route_at` and (unit) an unknown value
- [x] 2.2 Tagging: every text field of a non-general row starts with the tag in browse, search (keyword, semantic, hybrid) and the report's protected-origin threads; general rows untagged; stored text and `search_text` unchanged; idempotent (F5); exhaustive destructuring and a field-walking unit test (F4)
- [x] 2.3 `src/services/hubApi.ts`: optional `route` on the row types
- [x] 2.4 Message-search snippets of a tagged message start with the tag; tests with `protected`, `journal`, `cchv` and a far word (C1)
- [x] 2.5 `route=general` filter on browse and the journal leg, every mode; other values `400` (C7 step 1)
- [x] 2.6 Hub-produced fixture `scripts/fixtures/journal-read-shapes.json`, drift-checked (C4)

## 3. Distiller (tests, and the strip)

- [x] 3.1 With the tag as a content term, from the hub fixture: pretty JSON, compact JSON, `jq -r` of one field, a list-of-blocks tool result, a head-cut preview, a search hit and a report → protected, class `text_term`; each general twin → general; null and unknown routes tagged → protected; a split highlight and a hard wrap documented as general on the tag alone
- [x] 3.2 Paraphrase cases (C3): same session, another session of the group, a later day of the session → protected
- [x] 3.3 The distiller's tag equals the hub's and the routing term matches it; offered thread wording is stripped of the tag (F5)

## 4. Release

- [x] 4.1 The client's manager adds the tag to the routing file; cchv re-records the floor and sends the sha256 (done 2026-10-31 16:45Z, sha256 53db3f2e…); `scripts/journal-tag-probe.py` against the live file at release
- [ ] 4.2 Ships with `distill-route-full-session` and `distill-thread-origin-filter` (a precondition, C3), on ac's word
- [ ] 4.3 C2: count-only scan of the pre-release window at release; pending groups pinned protected or re-pended; count to the client's manager
- [ ] 4.4 C5 + C7 step 2 in the release row as one decision for ac: the hand-off residual, or default-exclude after labelling
- [ ] 4.5 Relay part 3 (C8) to context: every session outside client work filters `route == "general"` (null is not general), and no guidance quotes the literal tag
