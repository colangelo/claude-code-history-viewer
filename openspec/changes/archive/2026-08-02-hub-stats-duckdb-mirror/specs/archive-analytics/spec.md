# archive-analytics — Delta

## MODIFIED Requirements

### Requirement: Per-session statistics endpoint

The hub SHALL expose `GET /v1/stats/sessions/{id}` returning a
`SessionTokenStats` for one session: the four token counts plus reasoning
tokens, total tokens, message count, first and last message time, the session
summary when present, and its most used tools.

The `{id}` segment SHALL accept either the session's archive row id or its
provider session UUID, resolved by the same rule as
`GET /v1/sessions/{id}/messages`. A caller MUST NOT have to know which of the two
identifiers a given route expects: a UUID that identifies no session SHALL be
reported as not found rather than as a malformed request.

#### Scenario: Session statistics are returned for a known session

- **WHEN** an authenticated client requests statistics for an existing session id
- **THEN** the hub responds `200` with that session's token counts, message count, and time bounds

#### Scenario: A session UUID resolves the same as a row id

- **WHEN** an authenticated client requests statistics using a session's UUID
- **THEN** the hub responds `200` with the same statistics as the equivalent row-id request

#### Scenario: Unknown session returns not found

- **WHEN** an authenticated client requests statistics for a session id that does not exist, in either accepted form
- **THEN** the hub responds `404`

### Requirement: Global statistics endpoint

The hub SHALL expose `GET /v1/stats/global` returning archive-wide aggregate
statistics as a `GlobalStatsSummary`: totals across projects, sessions, and
messages; token and cost totals; per-model and per-provider breakdowns; and
daily activity. The endpoint MUST accept optional inclusive `from` and `to`
date-window parameters, and MUST require a valid read token.

Responses SHALL be computed from the derived statistics mirror. The endpoint
SHALL be interactive: an unwindowed archive-wide request MUST complete in
well under a second on a mirror of the current archive's scale, rather than
scaling with the number of stored messages on every call.

#### Scenario: Authenticated request returns archive-wide totals

- **WHEN** an authenticated client GETs `/v1/stats/global`
- **THEN** the hub responds `200` with token totals, cost totals, per-model and per-provider breakdowns, and daily activity spanning the archive

#### Scenario: Date window restricts the aggregate

- **WHEN** an authenticated client GETs `/v1/stats/global` with `from` and `to`
- **THEN** only messages whose timestamp falls inside the inclusive window contribute to the response

#### Scenario: The default view is interactive

- **WHEN** an authenticated client requests the archive-wide statistics the webapp loads by default
- **THEN** the response completes in well under a second

#### Scenario: Unauthenticated request is rejected

- **WHEN** a client GETs `/v1/stats/global` without a valid read token
- **THEN** the hub responds `401` and returns no statistics

### Requirement: Usage totals deduplicated per provider message

Token and cost totals SHALL count each provider message once, even though one
assistant response is stored across several rows sharing a provider message id
and repeating an identical usage block. Deduplication SHALL identify a logical
message by its provider message id where present, falling back to the record
uuid and then the row id, and SHALL attribute usage to exactly one row of each
logical message.

Deduplication MUST NOT be applied to tool invocations: the rows sharing a usage
block carry different content blocks, so their tool calls are distinct events.

The usage-bearing row of a logical message SHALL be determined over the archive
as a whole rather than recomputed per requested window. A logical message whose
rows straddle a window boundary MAY therefore contribute its usage to neither
side of that boundary; this is bounded and negligible, and is preferred to
recomputing deduplication on every request.

#### Scenario: Repeated usage blocks count once

- **WHEN** several stored rows share one provider message id and repeat its usage block
- **THEN** the totals count that message's tokens exactly once

#### Scenario: Rows without a provider message id are not collapsed

- **WHEN** stored rows carry distinct record uuids and no provider message id
- **THEN** each contributes its own usage

#### Scenario: Tool counts are not deduplicated

- **WHEN** rows sharing a provider message id each carry distinct tool invocations
- **THEN** every invocation is counted
