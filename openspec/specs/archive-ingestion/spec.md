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

The schema SHALL store, for each message, the normalized queryable columns (identifiers, ordering, timestamp, type/role/model, **the provider message id**, token counts, cost, duration, sidechain flag), the normalized `content` as JSONB, a raw-fidelity `raw` JSONB (stored verbatim as supplied by the daemon — the normalized record in the MVP; byte-exact original-line passthrough is a planned enhancement, see the change's design.md), a flattened `search_text`, and a `text_search` `tsvector` derived from `search_text` for full-text search. Projects and sessions MUST be stored with machine provenance and the aggregates needed to browse them.

The provider message id is the assistant response identifier the provider
assigns (the Anthropic `msg_…` id), stored in an indexed `message_id` column and
`NULL` when the provider supplies none. It MUST be a first-class column rather
than a JSONB path, because usage deduplication is expressed over it. It is
distinct from `message_key`, which is a content-derived row-dedup key, and from
the surrogate row `id`.

#### Scenario: The raw record is stored verbatim

- **WHEN** a message is ingested
- **THEN** the stored `raw` JSONB round-trips without loss to the `raw` the daemon supplied

#### Scenario: Full-text vector is populated for searchability

- **WHEN** a message with textual content is ingested
- **THEN** its `text_search` vector is populated from `search_text` and matches a full-text query for a term contained in the content

#### Scenario: Provider message id is stored as a queryable column

- **WHEN** a message carrying a provider message id is ingested
- **THEN** that id is readable from the `message_id` column without parsing JSONB

#### Scenario: Messages without a provider message id are accepted

- **WHEN** a message from a provider that assigns no message id is ingested
- **THEN** the row is stored with `message_id` NULL and ingest succeeds

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

### Requirement: Fingerprint persistence and identity derivation

The projects upsert SHALL persist the fingerprint facts
(`git_root_commit`, `git_remote_url`, `git_worktree`) and derive
`identity_key` per the project-identity capability, re-normalizing the remote
defensively. A batch that omits fingerprint fields MUST NOT clobber
previously stored non-null values (COALESCE semantics), while changed facts
MUST update the row and re-derive `identity_key`. The schema change is an
additive versioned migration (`0003`): nullable columns on `projects`, an
index on `identity_key`, and the `project_identity_aliases` table (unique
path, audited principal, timestamps).

#### Scenario: Fingerprint lands on upsert

- **WHEN** an `IngestProject` with fingerprint facts is ingested for an existing project row
- **THEN** the row gains the facts and a derived `identity_key` without changing its primary key or any session/message rows

#### Scenario: Absent facts never erase

- **WHEN** a later batch for the same project omits fingerprint fields (old daemon, or transient capture failure)
- **THEN** the stored fingerprint and `identity_key` are retained

#### Scenario: Migration is additive

- **WHEN** migration `0003` runs on the live database
- **THEN** all existing rows remain valid with NULL fingerprints and every pre-identity query behaves unchanged

### Requirement: Tool and skill invocation extraction

Ingest SHALL extract each tool invocation from a message and persist it as a row
associating the invocation with its message, carrying the tool name, the skill
name when the invocation is a Claude `Skill` call, the subagent type when the
invocation is a Claude `Agent` call, and whether the invocation resulted in an
error. Extraction MUST happen at ingest so that tool statistics never require
scanning message JSONB at query time.

Extraction MUST be idempotent with the message upsert: re-ingesting a message
MUST NOT accumulate duplicate invocation rows.

#### Scenario: Tool invocations are extracted and stored

- **WHEN** a message containing tool invocations is ingested
- **THEN** one invocation row per tool call is persisted, each naming its tool and referencing the message

#### Scenario: Skill invocations record the skill name

- **WHEN** an ingested message invokes the `Skill` tool naming a specific skill
- **THEN** the stored invocation carries that skill name in addition to the tool name

#### Scenario: Agent invocations record the subagent type

- **WHEN** an ingested message invokes the `Agent` tool naming a subagent type
- **THEN** the stored invocation carries that subagent type in addition to the tool name

#### Scenario: Same-record results are flagged on the invocation

- **WHEN** an ingested message carries a top-level tool invocation whose result on that same record reports an error
- **THEN** the stored invocation is flagged as an error

### Requirement: Tool outcome extraction

Ingest SHALL additionally extract each tool *result* from a message and persist
it as a row carrying the identifier of the invocation it reports on and whether
that invocation errored. This is required because a tool invocation does not
carry its own outcome: the outcome arrives in a later message referencing the
invocation by id, so success cannot be determined from the invocation alone.

Extraction MUST be idempotent with the message upsert, and MUST NOT depend on
the invocation already being stored — the invocation and its result may be
ingested in either order, in the same batch or different ones.

#### Scenario: Tool results are extracted and stored

- **WHEN** a message containing tool results is ingested
- **THEN** one row per result is persisted, each carrying the invocation identifier it references and its error status

#### Scenario: Result ingested before its invocation is retained

- **WHEN** a tool result is ingested in a batch that does not contain the invocation it references
- **THEN** the result row is stored and later resolves against that invocation once it is ingested

#### Scenario: Re-ingest does not duplicate results

- **WHEN** a message carrying tool results is ingested twice
- **THEN** the number of result rows for that message is unchanged

#### Scenario: Re-ingest does not duplicate invocations

- **WHEN** a message that has already been ingested is ingested again
- **THEN** the number of invocation rows for that message is unchanged

#### Scenario: Messages without tool use produce no rows

- **WHEN** a message containing no tool invocations is ingested
- **THEN** no invocation rows are created for it

