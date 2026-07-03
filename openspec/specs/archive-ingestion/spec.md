# archive-ingestion Specification

## Purpose

The hub's ingest endpoint and Postgres persistence layer — the only component
that holds database credentials. It accepts authenticated, idempotent batched
ingests and stores them in a normalized + raw-fidelity + full-text-searchable,
pgvector-ready schema managed by versioned migrations.

## Requirements

### Requirement: Authenticated batched ingest endpoint

The hub SHALL expose a `POST /v1/ingest` endpoint that accepts a batch containing a machine identifier and collections of projects, sessions, and messages, and persists them to Postgres. The endpoint MUST require a valid bearer token; requests with a missing or invalid token MUST be rejected with `401`. The hub MUST be the only component that holds Postgres credentials.

#### Scenario: Valid batch is accepted

- **WHEN** an authenticated client POSTs a well-formed batch to `/v1/ingest`
- **THEN** the hub persists the records and responds with `200` and counts of rows inserted and skipped

#### Scenario: Missing or invalid token is rejected

- **WHEN** a client POSTs to `/v1/ingest` without a valid bearer token
- **THEN** the hub responds `401` and persists nothing

#### Scenario: Malformed batch is rejected without partial corruption

- **WHEN** an authenticated client POSTs a batch that fails validation
- **THEN** the hub responds `400`, and no partial subset of the invalid batch is persisted

### Requirement: Idempotent upsert with stable identity

Ingest SHALL be idempotent. Messages MUST be uniquely identified by `(machine_id, provider, session_id, message_key)`, where `message_key` is the provider message UUID when present and otherwise a content-derived key. Re-ingesting an already-stored message MUST NOT create a duplicate row.

#### Scenario: Re-ingesting the same batch creates no duplicates

- **WHEN** an identical batch is ingested twice
- **THEN** the message, session, and project row counts are the same after the second ingest as after the first

#### Scenario: Provider without stable UUID still deduplicates

- **WHEN** messages from a provider that lacks stable UUIDs are ingested twice
- **THEN** the content-derived `message_key` prevents duplicate rows

### Requirement: Normalized, raw-fidelity, and full-text storage

The schema SHALL store, for each message, the normalized queryable columns (identifiers, ordering, timestamp, type/role/model, token counts, cost, duration, sidechain flag), the normalized `content` as JSONB, a raw-fidelity `raw` JSONB (stored verbatim as supplied by the daemon — the normalized record in the MVP; byte-exact original-line passthrough is a planned enhancement, see the change's design.md), a flattened `search_text`, and a `text_search` `tsvector` derived from `search_text` for full-text search. Projects and sessions MUST be stored with machine provenance and the aggregates needed to browse them.

#### Scenario: The raw record is stored verbatim

- **WHEN** a message is ingested
- **THEN** the stored `raw` JSONB round-trips without loss to the `raw` the daemon supplied

#### Scenario: Full-text vector is populated for searchability

- **WHEN** a message with textual content is ingested
- **THEN** its `text_search` vector is populated from `search_text` and matches a full-text query for a term contained in the content

### Requirement: Session aggregate maintenance

On ingest the hub SHALL maintain session-level aggregates (message count, first/last message time, has-tool-use, has-errors) and project-level aggregates (session count, message count, last modified) so that browse queries do not require scanning all messages.

#### Scenario: Aggregates reflect newly ingested messages

- **WHEN** additional messages for an existing session are ingested
- **THEN** the session's message count and last-message-time are updated to include them

### Requirement: Schema is pgvector-ready without enabling it now

The schema SHALL be structured so that embeddings can be added later (a dedicated `message_embeddings` relation keyed by message, supporting more than one embedding model) WITHOUT a breaking migration to the `messages` table. The pgvector extension MUST NOT be required for this change to function.

#### Scenario: Hub operates without the pgvector extension

- **WHEN** the hub runs against a Postgres instance where the `vector` extension is not installed
- **THEN** ingest, search, and browse all function normally

### Requirement: Versioned migrations

Database schema SHALL be managed by versioned `sqlx` migrations applied from a `migrations/` directory. Applying migrations to an empty database MUST produce the complete schema, and migrations MUST be idempotent to re-apply.

#### Scenario: Fresh database migrates to full schema

- **WHEN** migrations are applied to an empty Postgres database
- **THEN** all required tables, indexes, and the FTS index exist and the hub starts successfully
