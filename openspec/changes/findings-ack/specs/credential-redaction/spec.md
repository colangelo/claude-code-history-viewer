## ADDED Requirements

### Requirement: Acknowledged findings stop paging

The hub SHALL record an acknowledgement for a finding in a separate table, with a
classification (`false_positive` or `handled`), the acknowledger and a note, without
changing the finding. The findings summary and health check SHALL leave acknowledged
findings out of `total`, `by_rule`, `messages_by_rule`, `flagged_messages_by_rule`,
`redacted` and `latest_detected_at`, and SHALL report them as `acked_by_rule`. The health
check SHALL answer 200 when every finding in its window is acknowledged.

#### Scenario: Acknowledging the only finding turns the check green
- **WHEN** one message holds a `prefix` finding and an operator acknowledges that message as `handled`
- **THEN** `/v1/healthz/findings` answers 200 with `total` 0 and `acked_by_rule["prefix"]` 3

#### Scenario: A new finding after an acknowledgement still pages
- **WHEN** a second message with a finding arrives after the first was acknowledged
- **THEN** `/v1/healthz/findings` answers 503

### Requirement: Acknowledging is guarded and idempotent

`hub findings ack` SHALL refuse, before writing, when it has no message or time selector,
when the selection matches more findings than `--max-findings`, or when its note contains
anything the detector flags. Acknowledging an acknowledged finding SHALL change nothing.

#### Scenario: A note that holds a secret is refused
- **WHEN** the note contains a key-format value
- **THEN** the command writes nothing and reports that the note was refused, without the value

#### Scenario: Re-running adds nothing
- **WHEN** the same acknowledgement runs twice
- **THEN** the second run reports every finding as already acknowledged and writes no row

### Requirement: Per-day message counts

With `days=N` (1 to 90), the findings summary and health check SHALL include `daily`: one
entry per UTC day for the last N days including today, oldest first, each with
`messages_by_rule` counted as the top-level `messages_by_rule` is (distinct messages,
unacknowledged, `api-request-blob` copies excluded). A day with no finding SHALL appear
with an empty map. Without `days`, `daily` SHALL be absent.

#### Scenario: A week of counts
- **WHEN** a client asks `/v1/findings/summary?days=7`
- **THEN** `daily` has 7 entries ending today, and a day with findings counts each message once per rule
