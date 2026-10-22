## ADDED Requirements

### Requirement: References are exempt from assign-strong

The detector SHALL NOT classify an `assign` hit as `assign-strong` when its value contains an
unexpanded reference: `${`, `$(`, `{{`, `#{`, or `%NAME%`. Such a hit SHALL stay `assign` with
its `reference` flag set. A value containing a placeholder (`xxx`, `changeme`, `example`,
`dummy`, `redacted`, `<…>`, `***`, case-insensitive) SHALL keep its `assign-strong`
classification, with its `placeholder` flag set.

#### Scenario: A strong-shaped reference is not redacted
- **WHEN** `assign-strong` is in redact mode and a secret-named key holds a 40-character value with all four classes that contains `${`
- **THEN** the text is unchanged and the finding is `assign` with `reference_hits` 1

#### Scenario: A placeholder next to a strong value is still redacted
- **WHEN** `assign-strong` is in redact mode and the strong value contains `example`
- **THEN** the value is replaced by `[REDACTED:assign-strong]` and the finding has `placeholder_hits` 1

### Requirement: Flagged message counts per rule

The findings summary and health body SHALL include `flagged_messages_by_rule`: per rule, the
distinct messages with at least one reference hit and with at least one placeholder hit in the
window, excluding messages of type `api-request-blob`.

#### Scenario: A placeholder hit is counted apart
- **WHEN** one message has an `assign-strong` hit with a placeholder and another has one without
- **THEN** `messages_by_rule["assign-strong"]` is 2 and `flagged_messages_by_rule["assign-strong"]["placeholder"]` is 1
