## ADDED Requirements

### Requirement: Excluded project paths never leave the machine

The daemon SHALL accept `project_path_exclude`, a list of shell globs matched against each
project's decoded path. A matching project SHALL be skipped before any of its sessions is
parsed, uploaded or recorded in the checkpoint, and each pass SHALL count such projects.
An invalid glob SHALL stop the daemon at start-up.

#### Scenario: A canary project stays on the guest
- **WHEN** `project_path_exclude = ["/home/agent/client*"]` and sessions exist under `/home/agent/client-canary` and `/home/agent/infra`
- **THEN** only the `/home/agent/infra` session reaches the hub, and the pass reports one excluded project

#### Scenario: A typo does not fail open
- **WHEN** a pattern does not parse as a glob
- **THEN** the daemon refuses to start and names the pattern
