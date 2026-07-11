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

All read endpoints SHALL require authentication — either a valid bearer
token, or, when the hub is configured with a non-empty
`trust_tailscale_identity` allow-list, a `Tailscale-User-Login` request
header whose value matches an allow-listed identity (as injected by
Tailscale serve for tailnet clients). The identity path grants READ scope
only: `/v1/ingest` SHALL keep requiring a bearer token, since writes bind to
a machine identity. With the allow-list unset or empty, behavior is
unchanged (bearer only). All read endpoints SHALL support bounded pagination
via limit and offset, returning a stable order so that paging does not drop
or duplicate rows. Truncation MUST be detectable: the session messages
endpoint SHALL report the session's total message count in an
`X-Total-Count` response header, so a client that receives a default-limit
page (50; max 200) can tell it is partial.

#### Scenario: Unauthenticated read is rejected

- **WHEN** a client calls any read endpoint without a valid bearer token and without a trusted identity header
- **THEN** the hub responds `401`

#### Scenario: Trusted Tailscale identity is accepted for reads

- **WHEN** the hub is configured with `trust_tailscale_identity` containing a login, and a read request carries a `Tailscale-User-Login` header with that login and no bearer token
- **THEN** the request is served

#### Scenario: Untrusted identity is rejected

- **WHEN** a read request carries a `Tailscale-User-Login` header whose value is not in the allow-list (or the allow-list is empty)
- **THEN** the hub responds `401`

#### Scenario: Ingest ignores identity headers

- **WHEN** a request to `/v1/ingest` carries a trusted `Tailscale-User-Login` header but no valid bearer token
- **THEN** the hub responds `401`

#### Scenario: Paging is stable

- **WHEN** a client pages through a large result set using limit and offset
- **THEN** each row appears in exactly one page and the overall order is consistent across pages

### Requirement: Health endpoint

The hub SHALL expose an unauthenticated `GET /v1/healthz` endpoint that reports whether the service and its database connection are operational, suitable for liveness checks by the daemon and for deployment monitoring.

#### Scenario: Healthz reflects database connectivity

- **WHEN** the hub can reach Postgres
- **THEN** `GET /v1/healthz` responds `200` with a healthy status

### Requirement: Search scope and journal results

The `GET /v1/search` endpoint SHALL accept a `scope` parameter with values
`all` (default), `messages`, and `journal`. When scope includes journal, the
response SHALL carry a `journal` array — ranked full-text matches over journal
entries (headline, summary, topics, open_questions), each with its entry date,
project, and session ids — **alongside**, not mixed into, the existing
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
