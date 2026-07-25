# hub-stats-mirror — Delta

## ADDED Requirements

### Requirement: Derived statistics mirror

The hub SHALL maintain a local DuckDB file as a derived read model for
statistics, holding a projection of the archive sufficient to compute every
`/v1/stats/*` response: the message columns the rollups read, the session-to-
project mapping, projects, and the tool invocation and outcome tables.

Postgres SHALL remain the system of record. The mirror MUST be derivable from
Postgres alone, so that deleting the file and rebuilding it produces identical
statistics. The hub MUST NOT treat the mirror as an authority for anything other
than statistics, and MUST NOT write archive data to it from any path other than
the refresh.

The mirror's location SHALL be configurable, defaulting to a path under the hub's
configuration directory, and the hub SHALL apply an explicit memory limit to
DuckDB so statistics cannot exhaust a host shared with other jobs.

#### Scenario: Statistics are served from the mirror

- **WHEN** an authenticated client requests any `/v1/stats/*` endpoint and the mirror is ready
- **THEN** the response is computed from the mirror without querying Postgres

#### Scenario: A deleted mirror is rebuilt to the same answers

- **WHEN** the mirror file is deleted and rebuilt from Postgres
- **THEN** the statistics it produces are identical to those produced before deletion

### Requirement: Incremental append-only refresh

The hub SHALL refresh the mirror on a configurable interval by appending only
those rows whose primary key exceeds the highest key already mirrored, and SHALL
compute derived per-row values for the newly appended rows only.

Refreshes MUST NOT recompute previously mirrored rows. This is sound because the
deduplication marker records whether a row holds the lowest key within its
logical-message group, and keys are monotonic, so an appended row can never
become the lowest key of a group that already exists. A row carrying an older
timestamp under a newer key — as produced by historical backfill — MUST therefore
be appended without disturbing the group it joins.

A refresh MUST NOT run concurrently with another refresh; a scheduled refresh
that arrives while one is in progress SHALL be skipped rather than queued.

#### Scenario: New messages appear in statistics after a refresh

- **WHEN** messages are ingested and the next refresh completes
- **THEN** those messages contribute to subsequent statistics responses

#### Scenario: Backfilled history does not corrupt deduplication

- **WHEN** a row with an older timestamp and a newer key is appended into an existing logical-message group
- **THEN** it is not marked as that group's usage-bearing row, and the group's totals are unchanged

### Requirement: Warming state while no mirror exists

When no usable mirror exists — first start, or a mirror that could not be opened
— the hub SHALL respond to `/v1/stats/*` with `503` and a `Retry-After` header
while it builds one in the background, rather than blocking the request or
serving partial statistics.

An unreadable or corrupt mirror MUST be preserved by moving it aside under a
timestamped name before a replacement is built; it MUST NOT be deleted.

#### Scenario: Statistics are unavailable while warming

- **WHEN** a client requests statistics before the first mirror has been built
- **THEN** the hub responds `503` with `Retry-After` and no statistics body

#### Scenario: Statistics become available once built

- **WHEN** the background build completes
- **THEN** subsequent requests are served normally without a restart

### Requirement: Refresh failures degrade rather than escalate

A refresh that fails — including because Postgres is unreachable — SHALL leave
the existing mirror intact and the hub SHALL continue serving statistics from it.

The refresh path MUST NOT terminate the process and MUST NOT contribute to the
sustained-authentication-failure count that governs process exit, so that a
refresh failure is never mistaken for a rotated credential.

#### Scenario: Statistics survive a database outage

- **WHEN** Postgres is unreachable and a scheduled refresh fails
- **THEN** the hub continues to answer `/v1/stats/*` from the existing mirror

#### Scenario: A failing refresh does not restart the hub

- **WHEN** refreshes fail repeatedly
- **THEN** the process does not exit as a result

### Requirement: Mirror staleness is reported and monitorable

Every `/v1/stats/*` response SHALL carry headers reporting when the mirror was
last refreshed and how old it is. Staleness MUST be reported on headers rather
than in the response body, so the shared statistics types are unchanged.

The hub SHALL additionally expose an unauthenticated `GET /v1/healthz/stats`,
consumable by HTTP monitors that read only status code and body, reporting mirror
readiness and age and returning a non-success status when the mirror has not
refreshed within a configurable threshold.

#### Scenario: A served response reports its own staleness

- **WHEN** a client receives a successful statistics response
- **THEN** the response carries the mirror's last-refresh time and age

#### Scenario: A stalled mirror is visible to monitoring

- **WHEN** the mirror has not refreshed within the configured threshold
- **THEN** `GET /v1/healthz/stats` reports the staleness with a non-success status
