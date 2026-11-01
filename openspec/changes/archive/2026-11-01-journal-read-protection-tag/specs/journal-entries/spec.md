## ADDED Requirements

### Requirement: Protected journal text is tagged on read
Every journal read response SHALL start each text field (headline, summary, each topic, each open
question) of an entry whose counted route is not `general` with the tag
`[cchv:protected-journal] `. The stored row, its search text and its embedding SHALL stay untagged.

#### Scenario: Protected entry in a browse response
- **WHEN** a client browses a project whose entry has route `protected`
- **THEN** that entry's headline, summary, topics and open questions each start with the tag

#### Scenario: Entry without a route
- **WHEN** an entry has no route
- **THEN** its text fields are tagged as a protected entry's are

#### Scenario: General entry
- **WHEN** an entry has route `general`
- **THEN** none of its text fields carries the tag

#### Scenario: A transcript quoting a tagged field routes protected
- **WHEN** the tag is a content term in the routing file and a lab session's tool result holds a tagged field
- **THEN** that session's group is protected with class `text_term`

## MODIFIED Requirements

### Requirement: Journal browse endpoint

The hub SHALL expose `GET /v1/journal/entries` (read-auth) listing entries
filterable by project and date range, newest-first, paginated, returning full
entry content including session ids so a caller can drill into the underlying
transcripts, and each entry's counted `route` (`general`, `protected` or null). With
`route=general` it SHALL return only entries whose counted route is `general`.

#### Scenario: General-only browse

- **WHEN** a client browses with `route=general`
- **THEN** only entries whose counted route is `general` are returned

#### Scenario: Browse a project's journal

- **WHEN** an authenticated client lists journal entries filtered to one
  project over a date range
- **THEN** matching `entry`-status rows are returned newest-first with
  headline, summary, topics, open_questions, session ids and route
