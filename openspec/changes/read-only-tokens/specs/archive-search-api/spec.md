## ADDED Requirements

### Requirement: Read-only machine tokens

A token entry in the hub configuration MAY be marked `read_only`. A read-only token SHALL be
accepted by every read endpoint and SHALL be refused with 401 by every request whose method is not
GET, HEAD or OPTIONS, including `POST /v1/ingest`, the journal writes and the identity alias
writes. The refusal SHALL hold for write routes added later without per-route code. A token entry
marked both `read_only` and `ingest_only` SHALL make the configuration fail to load. Token entries
without the flag SHALL keep their scope.

#### Scenario: Read-only token reads
- **WHEN** a client calls a read endpoint (projects, sessions, session messages, search, findings
  summary, journal, open questions, stats, identities) with a read-only token
- **THEN** the hub answers as it does for a full-scope token

#### Scenario: Read-only token cannot ingest
- **WHEN** a client posts a batch to `/v1/ingest` with a read-only token
- **THEN** the hub answers 401 and stores nothing

#### Scenario: Read-only token cannot write the journal or aliases
- **WHEN** a client posts to `/v1/journal/entries`, `/v1/journal/ticks` or
  `/v1/identities/aliases`, or deletes `/v1/identities/aliases/{id}`, with a read-only token
- **THEN** the hub answers 401 and writes nothing

#### Scenario: A later write route refuses it too
- **WHEN** a client sends a read-only token with a method other than GET, HEAD or OPTIONS to any
  path, registered or not
- **THEN** the hub answers 401 before any handler runs

#### Scenario: Both flags is an error
- **WHEN** a token entry sets both `read_only` and `ingest_only`
- **THEN** loading the configuration fails and names the entry's label or machine id

#### Scenario: Other tokens are unchanged
- **WHEN** a token entry has no `read_only` flag
- **THEN** it reads, writes and ingests exactly as before
