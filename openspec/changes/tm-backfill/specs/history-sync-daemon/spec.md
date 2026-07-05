## MODIFIED Requirements

### Requirement: At-least-once batched delivery

The daemon SHALL deliver records to the hub in bounded batches and retry failed deliveries with backoff. Batches MUST be bounded both by message count and by serialized payload size (default 8 MiB, overridable via `CCHV_INGEST_MAX_BATCH_BYTES`), so that sessions containing very large messages do not produce requests exceeding the hub's body limit; a single message larger than the byte budget is still sent (alone), leaving the hub as the arbiter of a hard reject. Because the hub ingest is idempotent, re-delivery of an already-stored batch MUST NOT create duplicates. The daemon MUST advance a file's checkpoint only after the hub acknowledges the corresponding batch.

#### Scenario: Transient hub failure is retried and eventually delivered

- **WHEN** an ingest POST fails transiently and then succeeds on retry
- **THEN** the records are stored exactly once and the checkpoint advances only after the successful acknowledgement

#### Scenario: Session with huge messages is split under the body limit

- **WHEN** a session's messages would serialize to more than the byte budget in a single count-bounded batch
- **THEN** the daemon splits delivery into multiple smaller batches, none exceeding the byte budget (except a lone oversized message), and every message is delivered

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
