## ADDED Requirements

### Requirement: Stored record keeps the working folder

The normalized Claude Code message record the daemon stores as `raw` SHALL carry the source record's `cwd` when the source has one. Messages ingested before this change are not re-ingested and keep no working folder.

#### Scenario: A source line with a working folder
- **WHEN** the daemon converts a Claude Code JSONL line that has `cwd`
- **THEN** the stored `raw` holds that `cwd`

#### Scenario: A source line without one
- **WHEN** the line has no `cwd`
- **THEN** the stored `raw` has no `cwd` and is otherwise unchanged
