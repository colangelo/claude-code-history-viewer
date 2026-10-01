## ADDED Requirements

### Requirement: Session rows distinguish records from conversation items

Each row of the sessions list SHALL carry `message_count`, the number of **records**
archived for the session (every row in `messages`, including content-less state records
such as `attachment`, `mode` or `permission-mode`), and `conversation_count`, the number
of those records whose `content` is present. `message_count` MUST keep its meaning;
`conversation_count` is additive. The hub MUST compute `conversation_count` per returned
row from the session's own records, not by aggregating across sessions.

#### Scenario: State records are counted as records but not as conversation

- **WHEN** a session holds N records with content and M content-less records (at least one of type `attachment`) and `/v1/sessions` is queried
- **THEN** that session's row reports `conversation_count == N` and `message_count == N + M`

#### Scenario: A session with no state records reports equal counts

- **WHEN** every record of a session has content
- **THEN** `conversation_count == message_count` for that row
