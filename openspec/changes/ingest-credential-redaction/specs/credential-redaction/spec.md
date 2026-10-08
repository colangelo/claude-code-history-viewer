## Purpose

Detect credential-shaped values in archived transcripts as they are ingested, record each
detection without the value, and, per rule, keep the value from ever being stored, so a
leaked secret is caught by the archive instead of depending on the agent that leaked it.

## ADDED Requirements

### Requirement: Credential-shape detection at ingest
The hub SHALL scan every ingested message's `raw`, `content` and `search_text` against the
enabled credential rules before anything else is derived from the message.

#### Scenario: Private key in a tool result
- **WHEN** a message whose tool result contains a PEM `PRIVATE KEY` block is ingested
- **THEN** one finding with rule `pem` is recorded for that message

#### Scenario: Token counts are not credentials
- **WHEN** a message containing `"input_tokens": 1234` and `max_tokens=4096` is ingested
- **THEN** no finding is recorded for it

### Requirement: Findings never contain the value
A finding SHALL record the message, the field, the rule id, the detector version and the
detection time. It MUST NOT store, log, hash or return any part of the detected value.

#### Scenario: Finding row content
- **WHEN** a credential is detected
- **THEN** the finding row contains no substring of the detected value and no digest of it

### Requirement: Per-rule flag or redact mode
Each rule SHALL run in either `flag` mode (record the finding, store the message unchanged)
or `redact` mode (record the finding and replace the value before storage). Mode is
configured per rule in the hub configuration.

#### Scenario: Flag mode stores unchanged
- **WHEN** a rule in `flag` mode matches a message
- **THEN** a finding is recorded and the stored message is unchanged by that rule

#### Scenario: Redact mode stores the marker
- **WHEN** a rule in `redact` mode matches the value of `NATS_PASSWORD=<value>`
- **THEN** the stored `raw`, `content` and `search_text` contain
  `NATS_PASSWORD=[REDACTED:assign]` and no part of the value

### Requirement: Redaction is length-hiding and idempotent
The redaction marker SHALL be the same for every value a rule matches, and re-ingesting the
same message SHALL produce the same stored text and no duplicate finding.

#### Scenario: Re-sent batch
- **WHEN** a batch containing a redacted message is sent again
- **THEN** the stored row and the finding count are unchanged

### Requirement: Findings summary
The hub SHALL serve counts of findings by rule over a requested time window, without any
message text, so an external check can alert on new findings.

#### Scenario: New finding in the window
- **WHEN** a finding was recorded in the last 24 hours and the summary is requested with
  `since=24h`
- **THEN** the response reports a count ≥ 1 for that rule and contains no message text

### Requirement: Retroactive redaction of stored rows
The hub SHALL provide an operator command that applies the same rules to messages already
stored. A dry run SHALL change nothing and report each hit's rule, key name, location
(session, message, machine, timestamp) and value shape, never a value. A real run SHALL
rewrite matching rows and invalidate every copy derived from them.

#### Scenario: Dry run reports locations, not values
- **WHEN** the command runs with `--dry-run`
- **THEN** each hit is reported with its rule, key name, session id, message id, machine and
  timestamp, nothing is changed, and no value is printed

#### Scenario: Real run invalidates derived copies
- **WHEN** the command redacts a stored message
- **THEN** that message's search vector, tool rows and embeddings no longer contain the value,
  and its session's journal day is marked for re-distillation

### Requirement: Dry run proves it can see before it reports
The dry run SHALL accept expected hit locations and SHALL report the verdict "could not look"
instead of a result when any expected location is not found. Every report SHALL state what
it scanned and what it could not reach.

#### Scenario: A known leak is missed
- **WHEN** the dry run is given `--expect` for a message known to contain a credential and
  reports no hit at that location
- **THEN** its verdict is "could not look", not a count or "clean"

#### Scenario: Reach is stated
- **WHEN** any dry run completes
- **THEN** its report lists the machines, time range, fields, rules and detector version it
  scanned, and names what it could not reach

### Requirement: Rules are proven against fixtures
Every rule SHALL have at least one known-positive and one known-negative fixture, and the
test suite SHALL fail if any known-positive fixture stops matching.

#### Scenario: A rule regresses
- **WHEN** a change to a rule makes it miss its known-positive fixture
- **THEN** the test suite fails, naming the rule and the fixture, without printing the
  fixture value
