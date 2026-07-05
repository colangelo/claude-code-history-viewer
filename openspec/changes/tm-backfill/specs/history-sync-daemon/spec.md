## ADDED Requirements

### Requirement: Single-pass execution mode

The daemon SHALL support a `--once` command-line flag that performs exactly one full sync pass and then exits, without starting the file watcher or the periodic rescan loop. The process exit code MUST be `0` when the pass completed with zero errors and non-zero when the pass recorded any error, so callers can script against the outcome.

#### Scenario: One pass then clean exit

- **WHEN** the daemon is started with `--once` against a readable history directory and a reachable hub
- **THEN** it runs a single sync pass, delivers the discovered records, logs the pass stats, and exits with code `0`

#### Scenario: Errors surface in the exit code

- **WHEN** a `--once` pass records one or more errors (e.g. a session fails to parse or an ingest batch ultimately fails)
- **THEN** the process exits with a non-zero code after completing the pass

### Requirement: Hostname override for foreign-machine attribution

The daemon SHALL honor a `CCHV_HOSTNAME` environment variable that overrides the hostname reported in its machine identity. When the variable is unset or empty, the daemon MUST fall back to the system hostname. This allows history restored from another machine's backups to be attributed to the source machine (combined with a dedicated state directory carrying that machine's persistent id).

#### Scenario: Override replaces the system hostname

- **WHEN** the daemon runs with `CCHV_HOSTNAME=ac-mbp`
- **THEN** every batch it delivers carries hostname `ac-mbp` regardless of the host it runs on

#### Scenario: Unset override falls back to the system hostname

- **WHEN** the daemon runs without `CCHV_HOSTNAME` set
- **THEN** batches carry the system hostname, matching existing behavior
