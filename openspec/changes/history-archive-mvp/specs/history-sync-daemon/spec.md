## ADDED Requirements

### Requirement: Initial backfill of all local history

On first run against an empty checkpoint, the daemon SHALL enumerate every supported provider via `history-core`, read all projects, sessions, and messages currently present on disk, and deliver them to the hub. Backfill MUST be resumable: if interrupted, a subsequent run continues from the last acknowledged checkpoint rather than restarting from zero.

#### Scenario: Cold start sends everything once

- **WHEN** the daemon starts with no prior checkpoint against a populated history directory
- **THEN** every discovered message is delivered to the hub exactly as a batched ingest, and the local checkpoint records each session file as synced

#### Scenario: Interrupted backfill resumes

- **WHEN** a backfill is interrupted partway and the daemon is restarted
- **THEN** already-acknowledged session files are not re-scanned from scratch and the remaining files are processed to completion

### Requirement: Incremental synchronization

After backfill the daemon SHALL keep the archive current incrementally. For append-only JSONL sources it MUST track the last byte offset per file and parse only newly appended lines. For rewritten or database-backed sources (e.g. patch-log and SQLite providers) it MUST re-parse the affected session and send only records not already delivered, identified by message key. The daemon MUST also run a periodic safety-net full rescan to catch changes missed by the file watcher.

#### Scenario: Appended JSONL lines sync without re-reading the whole file

- **WHEN** new lines are appended to a previously synced JSONL session file
- **THEN** the daemon parses only the appended bytes from the stored offset and delivers only the new messages

#### Scenario: Rewritten session re-diffs and sends only new records

- **WHEN** a database-backed or patch-log session is rewritten with additional messages
- **THEN** the daemon re-parses the session and delivers only messages whose key was not previously delivered

#### Scenario: Safety-net rescan catches a missed change

- **WHEN** a file change occurs that the watcher did not surface and the periodic rescan interval elapses
- **THEN** the rescan detects the divergence from the checkpoint and delivers the missing messages

### Requirement: Crash-safe local checkpoint

The daemon SHALL persist a local checkpoint that records, per session file, at least its size/offset, message count, a content hash, and last-synced timestamp. The checkpoint MUST be durable across restarts so the daemon never re-sends already-acknowledged data unnecessarily and never loses track of un-synced data.

#### Scenario: Checkpoint survives restart

- **WHEN** the daemon is stopped and restarted with no new disk changes
- **THEN** it performs no redundant delivery because every tracked file matches its checkpoint

### Requirement: Stable machine identity and provenance

The daemon SHALL establish a stable machine identifier persisted in its local state directory and attach it (with hostname) to every batch, so archived records carry machine provenance. The identifier MUST remain constant across daemon restarts on the same machine.

#### Scenario: Same machine id across restarts

- **WHEN** the daemon runs, stops, and runs again on the same host
- **THEN** the machine identifier sent to the hub is identical across runs

### Requirement: At-least-once batched delivery

The daemon SHALL deliver records to the hub in bounded batches and retry failed deliveries with backoff. Because the hub ingest is idempotent, re-delivery of an already-stored batch MUST NOT create duplicates. The daemon MUST advance a file's checkpoint only after the hub acknowledges the corresponding batch.

#### Scenario: Transient hub failure is retried and eventually delivered

- **WHEN** an ingest POST fails transiently and then succeeds on retry
- **THEN** the records are stored exactly once and the checkpoint advances only after the successful acknowledgement

### Requirement: Cumulative archive semantics

The daemon SHALL treat the archive as cumulative. Deletion, truncation, or rotation of a local source file MUST NOT cause the daemon to delete or tombstone records previously delivered to the hub.

#### Scenario: Local deletion does not remove archived rows

- **WHEN** a source session file that was previously synced is deleted from local disk
- **THEN** the daemon issues no delete to the hub and the archived records remain intact and searchable

### Requirement: Configuration without database credentials

The daemon SHALL be configured with only a hub base URL and a bearer token (plus optional tuning such as scan interval and batch size). It MUST NOT require or hold Postgres credentials.

#### Scenario: Daemon runs with hub URL and token only

- **WHEN** the daemon is started with a hub URL and bearer token and no database configuration
- **THEN** it authenticates to the hub and synchronizes successfully
