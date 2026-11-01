## MODIFIED Requirements

### Requirement: Search scope and journal results

The `GET /v1/search` endpoint SHALL accept a `scope` parameter with values
`all` (default), `messages`, and `journal`. When scope includes journal, the
response SHALL carry a `journal` array — ranked full-text matches over journal
entries (headline, summary, topics, open_questions), each with its entry date,
project, session ids and counted route, its text fields tagged as journal reads
require — **alongside**, not mixed into, the existing
message `results` array. When `scope=messages`, the response MUST be exactly
the pre-journal response shape: no `journal` key, message hits unchanged.
Existing consumers that ignore unknown response fields MUST continue to work
without modification at any scope.

#### Scenario: Default scope returns both blocks

- **WHEN** an authenticated client searches with no `scope` parameter and the
  term matches both messages and journal entries
- **THEN** the response contains the existing `results` array of message hits
  and a `journal` array of entry hits, ranked independently

#### Scenario: scope=messages is byte-compatible

- **WHEN** a search is issued with `scope=messages`
- **THEN** the response shape is identical to the response before this
  capability existed (no `journal` key present)

#### Scenario: Journal-only search

- **WHEN** a search is issued with `scope=journal`
- **THEN** only journal entry hits are returned, and message search work is
  not performed

#### Scenario: Skip rows never surface

- **WHEN** any search matches text associated with a `skip`-status journal row
- **THEN** that row does not appear in results

#### Scenario: A message snippet from tagged text

- **WHEN** a message hit's stored text holds the protection tag, whatever word matched
- **THEN** its snippet starts with the tag

#### Scenario: General-only journal leg

- **WHEN** a search carries `route=general`
- **THEN** the journal block holds only entries whose counted route is `general`, in every mode, and any other value is rejected with `400`

#### Scenario: Protected journal hit

- **WHEN** a journal hit comes from an entry whose counted route is not `general`, in any search mode
- **THEN** the hit carries that route and each of its text fields starts with the protection tag
