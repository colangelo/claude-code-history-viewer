## MODIFIED Requirements

### Requirement: Normalized, raw-fidelity, and full-text storage

The schema SHALL store, for each message, the normalized queryable columns (identifiers, ordering, timestamp, type/role/model, **the provider message id**, token counts, cost, duration, sidechain flag), the normalized `content` as JSONB, a raw-fidelity `raw` JSONB (stored verbatim as supplied by the daemon — the normalized record in the MVP; byte-exact original-line passthrough is a planned enhancement, see the change's design.md — except for ingest sanitisation: NUL bytes are stripped, and a credential matched by a rule in `redact` mode is replaced by its redaction marker), a flattened `search_text`, and a `text_search` `tsvector` derived from `search_text` for full-text search. Projects and sessions MUST be stored with machine provenance and the aggregates needed to browse them.

The provider message id is the assistant response identifier the provider
assigns (the Anthropic `msg_…` id), stored in an indexed `message_id` column and
`NULL` when the provider supplies none. It MUST be a first-class column rather
than a JSONB path, because usage deduplication is expressed over it. It is
distinct from `message_key`, which is a content-derived row-dedup key, and from
the surrogate row `id`.

#### Scenario: The raw record is stored verbatim

- **WHEN** a message containing no NUL byte and no credential matched by a `redact`-mode rule is ingested
- **THEN** the stored `raw` JSONB round-trips without loss to the `raw` the daemon supplied

#### Scenario: Sanitised spans are the only difference

- **WHEN** a message containing a credential matched by a `redact`-mode rule is ingested
- **THEN** the stored `raw` equals the supplied `raw` except that each matched value is replaced by its redaction marker

#### Scenario: Full-text vector is populated for searchability

- **WHEN** a message with textual content is ingested
- **THEN** its `text_search` vector is populated from `search_text` and matches a full-text query for a term contained in the content

#### Scenario: Provider message id is stored as a queryable column

- **WHEN** a message carrying a provider message id is ingested
- **THEN** that id is readable from the `message_id` column without parsing JSONB

#### Scenario: Messages without a provider message id are accepted

- **WHEN** a message from a provider that assigns no message id is ingested
- **THEN** the row is stored with `message_id` NULL and ingest succeeds
