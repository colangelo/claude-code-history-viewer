## ADDED Requirements

### Requirement: Documentation example keys are not findings

The `prefix` rule SHALL NOT report a match that starts with `AKIA` and ends with
`EXAMPLE`. Any other `AKIA` key id SHALL still fire.

#### Scenario: The AWS docs example is left alone
- **WHEN** a message holds AWS's documentation example access key and a random `AKIA` key id
- **THEN** only the random key id is a finding, and in redact mode only it is replaced

### Requirement: Findings record where they sat

A finding from a JSON field SHALL carry the kind of the content block its value sat in:
`text` (text or thinking), `tool_result` (a tool_result block or `toolUseResult`),
`tool_input` (a tool_use block's `input`), `tool_use`, or `other`. Each
`credential_findings` row SHALL store the distinct kinds of its hits. The summary and
health body SHALL report `origins_by_rule` as distinct messages per `<role>:<kind>`,
counted as `messages_by_rule` is.

#### Scenario: A fetched page is told from typed text
- **WHEN** a user-role message holds a key-format value inside a tool_result block
- **THEN** its `content` and `raw` findings have origins `{tool_result}`, its `search_text` finding has none, and `origins_by_rule["prefix"]["user:tool_result"]` counts it

### Requirement: A bare PEM header is not a key

Text holding a PEM private-key header with no body and no END line SHALL produce no
finding under any rule.

#### Scenario: A docs sentence naming the header
- **WHEN** text says a key file starts with `-----BEGIN RSA PRIVATE KEY-----`
- **THEN** the detector reports nothing

### Requirement: Infra client keys are prefix findings

The `prefix` rule SHALL report `sk-bf-`, `sk-mw-`, `aiproxy-ws-` and `aiproxy-pi-` followed by
30 or more characters of `[A-Za-z0-9_:-]` as one match, colons included. It SHALL NOT report a
value whose tail after such a prefix is lowercase words and numbers joined by hyphens with an
optional lowercase extension (a file name).

#### Scenario: A key with a colon is redacted whole
- **WHEN** an `aiproxy-ws-` key whose tail holds a colon is in redact mode
- **THEN** the whole key, both sides of the colon, is replaced by one `[REDACTED:prefix]`

#### Scenario: A runbook file name is left alone
- **WHEN** text names `aiproxy-ws-rotation-runbook-for-the-new-workstations.md`
- **THEN** the detector reports nothing
