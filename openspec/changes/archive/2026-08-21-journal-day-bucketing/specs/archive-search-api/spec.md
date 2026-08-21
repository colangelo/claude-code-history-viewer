# archive-search-api (delta)

## MODIFIED Requirements

### Requirement: Browse and query endpoints

The hub SHALL expose read endpoints to browse the archive: list projects, list sessions (filterable by project, and by hub project row id via `project_id`), and retrieve the messages of a session. Response shapes SHOULD mirror the existing webui-server endpoints so a future phase can point the desktop viewer at the hub with minimal change.

The messages endpoint SHALL accept either the hub's surrogate session id or a provider session id (the `session_id` carried by search hits and session rows). A provider session id that matches sessions on more than one machine MUST be rejected with the candidate surrogate ids; an unknown session reference MUST be a `404`.

Messages SHALL be returned in chronological order (timestamp first, with seq and row id as tiebreaks; records without timestamps last). Ordering MUST NOT be seq-major: one archived session can aggregate several transcript files (subagent transcripts carry the parent session id), each with its own seq numbering from 0.

The messages endpoint SHALL additionally accept optional `from` and `to` timestamp
bounds, parsed as RFC 3339 with the same validation and error text as the search
endpoint's time filters, selecting the half-open window `[from, to)`. Either bound may
be given alone. When a bound is supplied, the `X-Total-Count` header MUST report the
count **within the window**, so that paging over a filtered result set is correct.
Records without timestamps fall outside any bounded window.

#### Scenario: List projects across machines

- **WHEN** an authenticated client requests the projects list
- **THEN** the hub returns archived projects with their machine provenance and aggregate counts

#### Scenario: Retrieve a session's messages in order

- **WHEN** an authenticated client requests the messages of a known session
- **THEN** the hub returns that session's messages in stable conversational order

#### Scenario: Retrieve one day of a long session

- **WHEN** a session spanning two days is requested with `from` and `to` bounding one
  logical day
- **THEN** only that day's messages are returned, and `X-Total-Count` reports the
  windowed count rather than the session total

#### Scenario: Malformed window bound is refused

- **WHEN** `from` or `to` is not a valid RFC 3339 timestamp
- **THEN** the hub responds `400` with the same error text the search endpoint uses

#### Scenario: Retrieve messages by provider session id

- **WHEN** an authenticated client requests `/v1/sessions/{id}/messages` using the session UUID from a search hit, and that UUID matches exactly one archived session
- **THEN** the hub returns that session's messages, without requiring a prior sessions-list lookup

#### Scenario: Ambiguous provider session id is refused with candidates

- **WHEN** the supplied provider session id matches sessions on more than one machine
- **THEN** the hub responds `400` with an error naming the candidate surrogate session ids

#### Scenario: Multi-file session reads chronologically

- **WHEN** a session's messages come from several transcript files whose seq numbering overlaps
- **THEN** the returned order is chronological, not interleaved by per-file seq

#### Scenario: Sessions filter by hub project id

- **WHEN** `/v1/sessions?project_id=<id>` is queried with a project row id from `/v1/projects`
- **THEN** only sessions of that project row (one machine + path) are returned; an unknown id yields an empty list, never the unfiltered one
