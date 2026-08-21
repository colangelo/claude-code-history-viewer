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
endpoint SHALL report the session's total message count, and the projects
and sessions lists their filtered totals, in an `X-Total-Count` response
header, so a client that receives a default-limit page (50; max 200) can
tell it is partial. Browse endpoints SHALL reject unknown query parameters
with `400` instead of ignoring them: an unsupported filter that silently
returns unfiltered rows reads exactly like a real answer.

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

#### Scenario: List truncation is detectable

- **WHEN** a projects or sessions list request matches more rows than the (clamped) limit
- **THEN** `X-Total-Count` carries the filtered total, so the caller can tell a capped page from the whole result set

#### Scenario: Unknown query params are rejected

- **WHEN** a browse endpoint is queried with a parameter it does not support (e.g. a typo of a real filter)
- **THEN** the hub responds `400` naming the unknown parameter, instead of returning plausible-looking unfiltered rows

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

### Requirement: Identity fields on projects listing

`GET /v1/projects` SHALL expose each project's `identity_key` (nullable) and
worktree flag so clients can group rows by identity without extra round
trips. Rows without a fingerprint carry NULL and group by path as before.

#### Scenario: Grouping data available in one call

- **WHEN** a client lists projects after two machines ingested clones of the same repo
- **THEN** both rows carry the same `identity_key` and the client can render them as one grouped project

### Requirement: Identity-scoped filters on search and browse

The project filter on `GET /v1/search` and `GET /v1/sessions` SHALL accept
the `identity:<key>` form defined by the project-identity capability
(server-side expansion to member + aliased paths), and both endpoints SHALL
honor `include_worktrees=false` in identity scope. Plain project filters and
all other parameters keep their existing semantics byte-for-byte.

#### Scenario: Search across a moved repo's whole history

- **WHEN** `/v1/search?q=…&project=identity:<key>` runs against an identity with a live path and an aliased dead path
- **THEN** hits from sessions under both paths are returned, ranked as one corpus

#### Scenario: Non-identity requests are byte-compatible

- **WHEN** any pre-identity request is replayed against the upgraded hub
- **THEN** the response is unchanged apart from the additive projects-listing fields

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

### Requirement: Journal search mode parameter

`GET /v1/search` SHALL accept a `mode` parameter
(`keyword` | `semantic` | `hybrid`, default `keyword`) governing the
journal block's retrieval per the semantic-search capability. The
message-results leg is unaffected by `mode` in this phase. An unknown
`mode` value is a 400; the default keeps every existing request
byte-compatible.

#### Scenario: Hybrid journal block

- **WHEN** `/v1/search?q=…&scope=journal&mode=hybrid` is queried on a hub with embeddings available
- **THEN** the journal block is the reciprocal-rank fusion of keyword and semantic rankings, message results are absent (scope), and the shape is unchanged

#### Scenario: Old clients unaffected

- **WHEN** any pre-change request (no `mode`) is replayed
- **THEN** the response is byte-identical to pre-change behavior
