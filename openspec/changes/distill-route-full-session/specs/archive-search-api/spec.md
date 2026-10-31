## ADDED Requirements

### Requirement: Message working folder

`GET /v1/sessions/:id/messages` SHALL return, for each message, a nullable `cwd` field holding the working folder recorded in the message's stored raw record, or null when the record has none.

#### Scenario: A message with a working folder
- **WHEN** a message was ingested from a record carrying `cwd`
- **THEN** the messages endpoint returns that value in `cwd`

#### Scenario: A message without one
- **WHEN** a message's raw record has no `cwd`
- **THEN** the messages endpoint returns `cwd: null` and every other field unchanged
