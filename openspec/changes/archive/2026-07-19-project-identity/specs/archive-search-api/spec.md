# archive-search-api Delta

## ADDED Requirements

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
