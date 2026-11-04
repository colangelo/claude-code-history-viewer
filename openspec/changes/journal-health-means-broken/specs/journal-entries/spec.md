## ADDED Requirements

### Requirement: Pending skips excluded paths

`GET /v1/journal/pending` SHALL NOT return a group whose `project_path` matches the journal exclusion patterns (built-in OS temp roots plus `[journal] exclude_paths`).

#### Scenario: Temp-path group is not offered
- **WHEN** a closed day has a pending group under `/private/var/folders/xy/abc/T/probe`
- **THEN** `pending` does not list it, and lists the day's other pending groups

### Requirement: Pause tick

`POST /v1/journal/ticks` SHALL accept `mode: "paused"` with a required `reason` of 1 to 500 characters, and SHALL reject a `reason` on any other mode with 400.

#### Scenario: Pause recorded
- **WHEN** a machine principal posts `{"mode": "paused", "groups_pending": 0, "reason": "usage brake"}`
- **THEN** the tick is stored with its reason and the response is 200

#### Scenario: Pause without reason refused
- **WHEN** the body is `{"mode": "paused", "groups_pending": 0}`
- **THEN** the response is 400 naming `reason`

### Requirement: Distiller drains, pauses and runs without connectors

The forward run SHALL re-read pending until it is empty, a time budget is spent, or a batch makes no progress. `cchv-distill --pause --reason TEXT` SHALL post one paused tick and exit. Every `claude -p` the distiller starts SHALL run with `ENABLE_CLAUDEAI_MCP_SERVERS=false`, plus `--settings ~/.config/claude/direct-noconnectors.settings.json` when that file exists.

#### Scenario: A busy day drains in one run
- **WHEN** 85 groups are pending at the start of a forward run and every group succeeds within the budget
- **THEN** the run processes all 85, reading pending more than once

#### Scenario: No progress stops the loop
- **WHEN** a batch finishes with zero successes
- **THEN** the run stops and exits non-zero as today on failures

#### Scenario: Model CLI starts without connectors
- **WHEN** the distiller starts `claude -p`
- **THEN** its environment has `ENABLE_CLAUDEAI_MCP_SERVERS=false`, and its argv has `--settings` with the no-connectors file when that file exists
