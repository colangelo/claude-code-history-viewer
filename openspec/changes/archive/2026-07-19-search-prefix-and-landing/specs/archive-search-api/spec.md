# archive-search-api Specification (delta)

## ADDED Requirements

### Requirement: Prefix matching for plain queries

For a plain query (no websearch operators: no quoted phrase, no `OR`, no
`-negation`), both search surfaces (message FTS and the journal block) SHALL
match word prefixes in addition to whole lexemes — `distill` matches
`distiller` and `distillation`. Queries using websearch syntax SHALL keep
exact websearch semantics with no prefix broadening. Prefix matching SHALL
only ever ADD hits relative to the previous behavior.

#### Scenario: Stem query finds derived forms

- **WHEN** `GET /v1/search?q=distill` runs against content containing only
  "distiller"
- **THEN** both the message results and the journal block return that content

#### Scenario: Advanced syntax stays exact

- **WHEN** the query contains a quoted phrase, `OR`, or a `-negated` term
- **THEN** results are identical to whole-lexeme websearch parsing

### Requirement: Hit position in session ordering

Each message search hit SHALL carry `position`: the 0-based index of the
message within its session's browse ordering (`timestamp ASC NULLS LAST,
seq ASC, id ASC`), exactly consistent with `GET /v1/sessions/{ref}/messages`
pagination — `floor(position / limit) * limit` is the offset of the page
containing the hit.

#### Scenario: Position indexes into the browse listing

- **WHEN** a hit has `position` P and the session's messages are fetched with
  a window covering P
- **THEN** the message at index P (relative to the session start) is the hit's
  message
