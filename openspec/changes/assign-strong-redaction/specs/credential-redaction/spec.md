## ADDED Requirements

### Requirement: Strong-shaped assignment values

The detector SHALL classify an `assign` hit as `assign-strong` when its value has 32+ characters
drawn from all four character classes (lower, upper, digit, symbol), or 64+ characters from at
least three. Redaction in `assign-strong` mode SHALL replace the value with the fixed marker
`[REDACTED:assign-strong]`. Selecting `assign` SHALL also select `assign-strong`.

#### Scenario: A strong value is redacted
- **WHEN** `assign-strong` is in redact mode and a secret-named key holds a 40-character value with all four classes
- **THEN** the value is replaced by `[REDACTED:assign-strong]` and a finding with rule `assign-strong` is recorded

#### Scenario: Weak shapes are untouched
- **WHEN** a secret-named key holds a 40-character hex digest, or a short value
- **THEN** the text is unchanged and any finding stays `assign`

### Requirement: Message counts per rule

The findings summary and health body SHALL include `messages_by_rule`: distinct messages with a
finding per rule in the window, excluding messages of type `api-request-blob`.

#### Scenario: A request copy is counted once
- **WHEN** a finding exists on a message and on its `api-request-blob` copy
- **THEN** `messages_by_rule` counts one message for that rule
