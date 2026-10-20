## ADDED Requirements

### Requirement: Ingest-only machine tokens

A token entry in the hub configuration MAY be marked `ingest_only`. An ingest-only token
SHALL be accepted by `POST /v1/ingest`, writing under its own machine id, and SHALL be refused
with 401 by every read endpoint and by every other machine-token write (journal entries,
distiller ticks). Token entries without the flag SHALL keep their full scope.

#### Scenario: Ingest-only token ingests
- **WHEN** a client posts a valid batch to `/v1/ingest` with an ingest-only token
- **THEN** the hub answers 200 and stores the batch under that token's machine id

#### Scenario: Ingest-only token cannot read
- **WHEN** a client calls any read endpoint (projects, sessions, search, findings summary,
  journal, stats) with an ingest-only token
- **THEN** the hub answers 401, as for an unknown token

#### Scenario: Ingest-only token cannot write the journal
- **WHEN** a client posts to `/v1/journal/entries` or `/v1/journal/ticks` with an ingest-only
  token
- **THEN** the hub answers 401 and writes nothing

#### Scenario: Full-scope tokens are unchanged
- **WHEN** a token entry has no `ingest_only` flag
- **THEN** it reads and ingests exactly as before
