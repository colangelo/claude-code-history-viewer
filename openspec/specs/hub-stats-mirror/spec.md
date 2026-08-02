# hub-stats-mirror Specification

## Purpose

The hub's derived read model for analytics: a local DuckDB projection of
the archive that every `/v1/stats/*` response is computed from, kept current by
an incremental refresh. Postgres remains the system of record; this capability
covers how the mirror is built, refreshed, rebuilt, degraded, and monitored.

## Requirements


### Requirement: Derived statistics mirror

The hub SHALL maintain a local DuckDB file as a derived read model for
statistics, holding a projection of the archive sufficient to compute every
`/v1/stats/*` response: the message columns the rollups read, the session-to-
project mapping together with the provider session identifiers, projects and
the project-identity grouping, and the tool invocation and outcome tables.

Every identifier a statistics endpoint accepts — session row id, session UUID,
project identity key — SHALL be resolvable from the mirror alone, so that
serving a statistics request never requires Postgres.

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

### Requirement: Incremental idempotent refresh

The hub SHALL refresh the mirror on a configurable interval by fetching rows
from an overlap window extending behind the mirrored high watermark — not only
rows strictly above it — and SHALL insert them idempotently, so a row already
mirrored is ignored rather than duplicated. Derived per-row values SHALL be
computed for newly appended rows only.

The overlap exists because rows do not become visible in key order: keys are
assigned at insert but transactions commit out of order under concurrent
ingest, so a row with a lower key can appear after the watermark has passed it.
A refresh that trusted the watermark alone would skip such rows permanently and
silently.

Refreshes MUST NOT recompute previously mirrored rows. This is sound for
inserts because the deduplication marker records whether a row holds the lowest
key within its logical-message group, and keys are monotonic, so an appended
row can never become the lowest key of a group that already exists. A row
carrying an older timestamp under a newer key — as produced by historical
backfill — MUST therefore be appended without disturbing the group it joins.

A refresh MUST NOT run concurrently with another refresh; a scheduled refresh
that arrives while one is in progress SHALL be skipped rather than queued.

#### Scenario: New messages appear in statistics after a refresh

- **WHEN** messages are ingested and the next refresh completes
- **THEN** those messages contribute to subsequent statistics responses

#### Scenario: Rows committed out of order are not skipped

- **WHEN** a row with a lower key becomes visible after rows with higher keys have already been mirrored
- **THEN** a subsequent refresh appends it exactly once

#### Scenario: Backfilled history does not corrupt deduplication

- **WHEN** a row with an older timestamp and a newer key is appended into an existing logical-message group
- **THEN** it is not marked as that group's usage-bearing row, and the group's totals are unchanged

### Requirement: Updates are out of contract; rebuild is the remedy

Incremental refresh SHALL cover inserted rows only. An operation that updates
mirrored columns of existing Postgres rows — `hub backfill-analytics`, which
rewrites `message_id` over existing rows, is the canonical example — SHALL
require a mirror rebuild to be reflected in statistics, and this SHALL be
documented alongside that operation's runbook.

The hub SHALL provide a rebuild subcommand that builds a fresh mirror aside and
atomically replaces the current one. While a rebuild runs against an intact
mirror, statistics SHALL continue to be served from the existing file until the
replacement is ready.

#### Scenario: A backfill's regrouping is reflected after rebuild

- **WHEN** existing rows' provider message ids are updated by a backfill and the rebuild subcommand is run
- **THEN** subsequent statistics reflect the updated grouping with no over-counted usage

#### Scenario: Rebuild does not interrupt serving

- **WHEN** a rebuild runs while an intact mirror exists
- **THEN** statistics requests continue to be answered from the existing mirror until the replacement swaps in

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
refreshed within a configurable threshold. The endpoint SHALL also report the
mirror's high watermark against the archive's current maximum row key, so an
advancing-but-incomplete or stuck watermark is observable and completeness is
monitorable rather than assumed — refresh age alone cannot distinguish a healthy
mirror from one that is silently skipping rows.

#### Scenario: A served response reports its own staleness

- **WHEN** a client receives a successful statistics response
- **THEN** the response carries the mirror's last-refresh time and age

#### Scenario: A stalled mirror is visible to monitoring

- **WHEN** the mirror has not refreshed within the configured threshold
- **THEN** `GET /v1/healthz/stats` reports the staleness with a non-success status

#### Scenario: A lagging watermark is visible to monitoring

- **WHEN** the mirror's high watermark falls behind the archive's maximum row key
- **THEN** `GET /v1/healthz/stats` reports the lag
