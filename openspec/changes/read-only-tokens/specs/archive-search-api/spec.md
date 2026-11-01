## ADDED Requirements

### Requirement: Read-only reader tokens

The hub configuration MAY list readers in a `[[readers]]` table, each with `token`,
`machine_id` and `label`. A reader token SHALL be accepted by every read endpoint and SHALL be
refused with 401 by every request whose method is not GET, HEAD or OPTIONS, including
`POST /v1/ingest`, the journal writes and the identity alias writes. The refusal SHALL hold for
write routes added later without per-route code. Readers live in their own table so that a hub
binary that predates them ignores the table and grants the token nothing (fail closed); a
`[[tokens]]` or `[[readers]]` entry with a field the hub does not know SHALL make the
configuration fail to load. Token entries keep their scope.

#### Scenario: Reader token reads
- **WHEN** a client calls a read endpoint (projects, sessions, session messages, search, findings
  summary, journal, open questions, stats, identities) with a reader token
- **THEN** the hub answers as it does for a full-scope token

#### Scenario: Reader token cannot ingest
- **WHEN** a client posts a batch to `/v1/ingest` with a reader token
- **THEN** the hub answers 401 and stores nothing

#### Scenario: Reader token cannot write the journal or aliases
- **WHEN** a client posts to `/v1/journal/entries`, `/v1/journal/ticks` or
  `/v1/identities/aliases`, or deletes `/v1/identities/aliases/{id}`, with a reader token
- **THEN** the hub answers 401 and writes nothing

#### Scenario: A later write route refuses it too
- **WHEN** a client sends a reader token with a method other than GET, HEAD or OPTIONS to any
  path, registered or not, with or without the static webapp mounted
- **THEN** the hub answers 401 before any handler runs

#### Scenario: An older hub fails closed
- **WHEN** a hub binary without reader support loads a configuration with a `[[readers]]` entry
- **THEN** it ignores the table and the reader token is refused everywhere

#### Scenario: An unknown field is an error
- **WHEN** a `[[tokens]]` or `[[readers]]` entry carries a field the hub does not know (for example
  `read_only = true` under `[[tokens]]`)
- **THEN** loading the configuration fails

#### Scenario: Duplicates are an error
- **WHEN** two entries across `[[tokens]]` and `[[readers]]` share a token string or a machine id
- **THEN** loading the configuration fails, naming the entries by label or machine id and never by
  token, and the launcher refuses to start the hub rather than reuse a cached configuration

#### Scenario: Other tokens are unchanged
- **WHEN** the configuration has no `[[readers]]` entry
- **THEN** every token reads, writes and ingests exactly as before
