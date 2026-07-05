# archive-search-api Specification

## Purpose

The hub's read API: full-text search and browse/query endpoints over the
archive, reachable (with a bearer token) from any machine, plus an unauthenticated
health endpoint for liveness checks.

## Requirements

### Requirement: Full-text search endpoint

The hub SHALL expose a `GET /v1/search` endpoint that performs Postgres full-text search over archived messages and returns ranked matches, each carrying enough session and project context to locate it. The endpoint MUST support filtering by provider, machine, project, and time range, and MUST support a free-text query.

#### Scenario: Query returns ranked matches

- **WHEN** an authenticated client searches for a term present in archived messages
- **THEN** the hub returns matching messages ordered by relevance, each including its session and project context

#### Scenario: Filters narrow results

- **WHEN** a search is issued with a provider and/or machine and/or project and/or time-range filter
- **THEN** only matches satisfying all supplied filters are returned

#### Scenario: No matches returns an empty result set

- **WHEN** a search term matches nothing in the archive
- **THEN** the hub returns an empty, well-formed result set with `200`, not an error

### Requirement: Browse and query endpoints

The hub SHALL expose read endpoints to browse the archive: list projects, list sessions (filterable by project), and retrieve the messages of a session. Response shapes SHOULD mirror the existing webui-server endpoints so a future phase can point the desktop viewer at the hub with minimal change.

The messages endpoint SHALL accept either the hub's surrogate session id or a provider session id (the `session_id` carried by search hits and session rows). A provider session id that matches sessions on more than one machine MUST be rejected with the candidate surrogate ids; an unknown session reference MUST be a `404`.

Messages SHALL be returned in chronological order (timestamp first, with seq and row id as tiebreaks; records without timestamps last). Ordering MUST NOT be seq-major: one archived session can aggregate several transcript files (subagent transcripts carry the parent session id), each with its own seq numbering from 0.

#### Scenario: List projects across machines

- **WHEN** an authenticated client requests the projects list
- **THEN** the hub returns archived projects with their machine provenance and aggregate counts

#### Scenario: Retrieve a session's messages in order

- **WHEN** an authenticated client requests the messages of a known session
- **THEN** the hub returns that session's messages in stable conversational order

#### Scenario: Retrieve messages by provider session id

- **WHEN** an authenticated client requests `/v1/sessions/{id}/messages` using the session UUID from a search hit, and that UUID matches exactly one archived session
- **THEN** the hub returns that session's messages, without requiring a prior sessions-list lookup

#### Scenario: Ambiguous provider session id is refused with candidates

- **WHEN** the supplied provider session id matches sessions on more than one machine
- **THEN** the hub responds `400` with an error naming the candidate surrogate session ids

#### Scenario: Multi-file session reads chronologically

- **WHEN** a session's messages come from several transcript files whose seq numbering overlaps
- **THEN** the returned order is chronological, not interleaved by per-file seq

### Requirement: Authentication and pagination

All read endpoints SHALL require a valid bearer token and SHALL support bounded pagination via limit and offset, returning a stable order so that paging does not drop or duplicate rows. Truncation MUST be detectable: the session messages endpoint SHALL report the session's total message count in an `X-Total-Count` response header, so a client that receives a default-limit page (50; max 200) can tell it is partial.

#### Scenario: Unauthenticated read is rejected

- **WHEN** a client calls any read endpoint without a valid bearer token
- **THEN** the hub responds `401`

#### Scenario: Paging is stable

- **WHEN** a client pages through a large result set using limit and offset
- **THEN** each row appears in exactly one page and the overall order is consistent across pages

### Requirement: Health endpoint

The hub SHALL expose an unauthenticated `GET /v1/healthz` endpoint that reports whether the service and its database connection are operational, suitable for liveness checks by the daemon and for deployment monitoring.

#### Scenario: Healthz reflects database connectivity

- **WHEN** the hub can reach Postgres
- **THEN** `GET /v1/healthz` responds `200` with a healthy status
