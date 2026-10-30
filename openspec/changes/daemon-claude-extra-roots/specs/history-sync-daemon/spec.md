## ADDED Requirements

### Requirement: Extra Claude config folders are archived like the main one

The daemon SHALL accept `claude_extra_roots`, a list of Claude config folders. Each existing
root SHALL be scanned as a Claude provider root (its `projects/`), with the same project path
decoding as `~/.claude`, and SHALL be watched for changes. When the Claude provider is
excluded, extra roots SHALL be skipped too.

#### Scenario: A helper's own account folder reaches the hub
- **WHEN** `claude_extra_roots = ["~/.config/claude-hlp-b"]` and a session exists under `~/.config/claude-hlp-b/projects/<project>/`
- **THEN** that session reaches the hub as provider `claude`, under the project its folder name decodes to

### Requirement: Extra roots are resolved safely at start-up

Roots SHALL be canonicalized at start-up. A root that is missing, relative, or resolves to the
main Claude folder or to an earlier listed root SHALL be skipped with a warning and SHALL NOT
stop the daemon.

#### Scenario: The same folder is never read twice
- **WHEN** an extra root is a symlink to the main Claude folder, or is listed twice
- **THEN** it is skipped with a warning and each session is sent once

#### Scenario: A missing folder does not stop the daemon
- **WHEN** an extra root does not exist
- **THEN** the daemon logs a warning and archives the remaining roots
