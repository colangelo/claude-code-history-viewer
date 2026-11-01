## ADDED Requirements

### Requirement: Message working folder

`GET /v1/sessions/:id/messages` SHALL return, for each message, a `cwd` field holding the working folder in the message's stored normalized record, or null when the record has none. The key SHALL always be present.

#### Scenario: A message with a working folder
- **WHEN** a message was ingested from a Claude Code record carrying `cwd` by a daemon with this change
- **THEN** the messages endpoint returns that value in `cwd`

#### Scenario: A message without one
- **WHEN** a message's stored record has no working folder (older ingests, other providers)
- **THEN** the messages endpoint returns `cwd: null` and every other field unchanged
