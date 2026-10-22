## ADDED Requirements

### Requirement: Structured name/value pairs

The detector SHALL record a `pair` finding for a value field that sits in the same object as a name field whose
content is secret-named, or in an object carrying a sensitivity flag set to true, in flat JSON text, XML
attributes, XML child elements, YAML blocks and structured JSON. A value SHALL NOT fire when it is shorter than 8
characters, a whole-value reference or placeholder, a boolean, all digits under 12 characters, a URL without
userinfo, or an existing mask. In `pair` redact mode every occurrence of a detected value in the text SHALL be
replaced by `[REDACTED:pair]`, and a value repeated in one text SHALL count once.

#### Scenario: A secret-named pair is redacted
- **WHEN** `pair` is in redact mode and a JSON variable list holds `"Name": "DB_PASSWORD", "Value": "<8+ chars>"`
- **THEN** the value is replaced by `[REDACTED:pair]` and one `pair` finding with basis `name` is recorded

#### Scenario: A flagged pair under a harmless name is redacted
- **WHEN** an object holds `Name: "BuildTarget"`, `Value: "<8+ chars>"` and `Sensitive: true`
- **THEN** the value is replaced by `[REDACTED:pair]` with basis `flag`

#### Scenario: Negative controls are untouched
- **WHEN** a pair is `PasswordMinLength` = `12`, or `PASSWORD_RESET_URL` = a URL without userinfo, or `Description` = long prose, or an unflagged harmless name, or an already masked value
- **THEN** the text is unchanged and no `pair` finding is recorded
