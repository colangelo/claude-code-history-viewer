## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: Tool and skill invocation extraction

Ingest SHALL extract each tool invocation from a message and persist it as a row
associating the invocation with its message, carrying the tool name, the skill
name when the invocation is a Claude `Skill` call, and whether the invocation
resulted in an error. Extraction MUST happen at ingest so that tool statistics
never require scanning message JSONB at query time.

Extraction MUST be idempotent with the message upsert: re-ingesting a message
MUST NOT accumulate duplicate invocation rows.

#### Scenario: Tool invocations are extracted and stored

- **WHEN** a message containing tool invocations is ingested
- **THEN** one invocation row per tool call is persisted, each naming its tool and referencing the message

#### Scenario: Skill invocations record the skill name

- **WHEN** an ingested message invokes the `Skill` tool naming a specific skill
- **THEN** the stored invocation carries that skill name in addition to the tool name

#### Scenario: Errored invocations are marked

- **WHEN** an ingested tool invocation produced an error result
- **THEN** the stored invocation is flagged as an error

#### Scenario: Re-ingest does not duplicate invocations

- **WHEN** a message that has already been ingested is ingested again
- **THEN** the number of invocation rows for that message is unchanged

#### Scenario: Messages without tool use produce no rows

- **WHEN** a message containing no tool invocations is ingested
- **THEN** no invocation rows are created for it
