## 1. Review

- [ ] 1.1 Review of the routing half by the client's manager (and its security reviewer if they ask), per #19
- [ ] 1.2 manager-lab's go to implement, after 1.1

## 2. Hub

- [ ] 2.1 `route` (counted) on `JournalEntry` and `JournalHit`, all search modes; tests for general, protected, null and a rewritten row
- [ ] 2.2 `protect_text`: every text field of a non-general row starts with the tag in browse, search (full-text, semantic, hybrid) and the report's protected-origin threads; a general row's fields do not; stored rows and `search_text` unchanged
- [ ] 2.3 `src/services/hubApi.ts`: optional `route` on the row types

## 3. Distiller (test only)

- [ ] 3.1 With the tag as a content term: a lab group whose only client trace is a tool result holding a browse response with one protected lab entry → protected, class `text_term`; its twin with a general entry → general; the same for a search hit and an open-questions report

## 4. Release

- [ ] 4.1 The client's manager adds the tag to the routing file; cchv re-records the floor and sends the sha256
- [ ] 4.2 Ships with `distill-route-full-session` and `distill-thread-origin-filter`, on ac's word
- [ ] 4.3 Relay part 3 (agent guidance: read only `route == "general"` outside client work) to context for `cchv-find`
