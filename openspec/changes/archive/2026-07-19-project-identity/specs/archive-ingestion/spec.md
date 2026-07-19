# archive-ingestion Delta

## ADDED Requirements

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
