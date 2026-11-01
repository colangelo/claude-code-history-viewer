## ADDED Requirements

### Requirement: Hex secrets under a secret key

The detector SHALL report `assign-hex-secret` for a key/value assignment whose key ends in
`TOKEN`, `SECRET`, `PASSWORD`, `PASSWD`, `PASSW`, `API_KEY`, `API-KEY`, `APIKEY` or `BEARER`
(case-insensitive) and whose value is 32 or more hexadecimal characters, unless the value is
already `assign-strong`. The rule SHALL redact only when listed in `[redaction] redact`.

#### Scenario: Positive controls
- **WHEN** text holds `hub_token = "<64 hex>"`, an env line `GITHUB_TOKEN=<40 hex>`, or a JSON
  field `"token": "<32 hex>"`
- **THEN** each is an `assign-hex-secret` finding

#### Scenario: Negative controls
- **WHEN** text holds `sha256 = <64 hex>`, `digest = <64 hex>`, `commit = <40 hex>`, hex under an
  unrelated key, a git sha in prose, or `token = <31 hex>`
- **THEN** none is an `assign-hex-secret` finding

### Requirement: Volume count leaves out false positives

`messages_by_rule_all` SHALL count, per rule, the distinct messages with a finding in the window,
leaving out only findings acked `false_positive`; unacked findings and findings acked `handled`
SHALL count.

#### Scenario: Ten hits
- **WHEN** 10 messages hit a rule in the window
- **THEN** the count is 10 while unacked, 0 once all are acked `false_positive`, and 10 when all
  are acked `handled`

### Requirement: Placeholder words

A value that is exactly one ALL_CAPS word starting `GENERATE_`, `EXAMPLE_`, `YOUR_` or `CHANGE_ME`
SHALL be a placeholder.

#### Scenario: Placeholder in a template
- **WHEN** text holds `token = "GENERATE_A_LONG_RANDOM_SECRET_FOR_M4M"`
- **THEN** no `assign-strong` finding is made
