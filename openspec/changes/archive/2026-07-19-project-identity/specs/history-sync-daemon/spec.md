# history-sync-daemon Delta

## ADDED Requirements

### Requirement: Git fingerprint capture at scan time

During each sync pass the daemon SHALL attempt to capture the git fingerprint
(root commit, normalized remote, worktree status per the project-identity
capability) for every discovered project whose resolved actual path contains a
`.git` marker, and send the facts as additive `IngestProject` fields.
Capture MUST be guarded: it runs only when a `.git` file-or-dir exists, each
git subprocess is time-limited (a hung or missing `git` binary degrades to
no-fingerprint), and a capture failure MUST NOT fail or delay the project's
session sync. Providers that expose no real filesystem path for a project
send no fingerprint.

#### Scenario: Fingerprint accompanies project ingest

- **WHEN** a sync pass scans a project at a git-repo path with `origin` set
- **THEN** the `IngestProject` sent to the hub carries root commit, normalized remote, and worktree status

#### Scenario: Capture failure degrades silently

- **WHEN** the git subprocess times out or errors for one project
- **THEN** that project ingests without fingerprint fields and the pass continues normally

#### Scenario: Old hub compatibility

- **WHEN** the daemon sends fingerprint fields to a hub that predates them
- **THEN** ingest succeeds (unknown fields ignored) with pre-identity behavior
