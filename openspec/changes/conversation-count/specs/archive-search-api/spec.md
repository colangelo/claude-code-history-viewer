## ADDED Requirements

### Requirement: Session rows carry a conversation-item count

Each row of the sessions list SHALL carry `conversation_count`, the number of the
session's stored rows whose `content` is present (content-less state records such as
`attachment`, `mode` or `permission-mode` are not conversation items). The hub MUST
compute it per returned row from that session's own rows, not by aggregating across
sessions. `message_count` keeps its existing meaning and value; `conversation_count` is
additive and MUST NOT change how `message_count` is produced or defined.

#### Scenario: State records are not counted as conversation items

- **WHEN** a session stores N rows with content and M content-less rows (at least one of type `attachment`) and `/v1/sessions` is queried
- **THEN** that session's row reports `conversation_count == N`
