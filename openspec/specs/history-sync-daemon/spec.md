# history-sync-daemon Specification

## Purpose

The per-machine sync daemon backfills and incrementally pushes local agent
history to the hub. It holds no database credentials, persists a stable machine
identity and a crash-safe checkpoint, delivers at-least-once, and treats the
archive as cumulative (a deleted local file never removes archived rows).

## Requirements

### Requirement: Initial backfill of all local history

On first run against an empty checkpoint, the daemon SHALL enumerate every supported provider via `history-core`, read all projects, sessions, and messages currently present on disk, and deliver them to the hub. Backfill MUST be resumable: if interrupted, a subsequent run continues from the last acknowledged checkpoint rather than restarting from zero.

#### Scenario: Cold start sends everything once

- **WHEN** the daemon starts with no prior checkpoint against a populated history directory
- **THEN** every discovered message is delivered to the hub exactly as a batched ingest, and the local checkpoint records each session file as synced

#### Scenario: Interrupted backfill resumes

- **WHEN** a backfill is interrupted partway and the daemon is restarted
- **THEN** already-acknowledged session files are not re-scanned from scratch and the remaining files are processed to completion

### Requirement: Incremental synchronization

After backfill the daemon SHALL keep the archive current incrementally. It MUST detect a changed session file (by comparing the file's size and mtime against the checkpoint), re-parse the changed session, and re-deliver its records; the hub's idempotent ingest drops already-stored messages, so re-delivery never produces duplicates. The daemon MUST run a periodic safety-net full rescan so that no change is missed. The daemon SHALL additionally watch the provider roots (recursive debounced file-watching, excluding providers in `providers_exclude`) and trigger an early sync pass on activity, throttled to at most one watcher-triggered pass per configured minimum gap; the watcher is a latency optimization only — it MUST degrade to rescan-only behavior on failure, and the rescan schedule MUST be independent of watcher-triggered passes. (Byte-offset "parse only the appended bytes" remains a future optimization, not required for correctness given hub-side dedup; see the change's design.md.)

#### Scenario: Filesystem activity triggers an early pass

- **WHEN** a watched session file changes while the daemon is idle between rescans
- **THEN** a sync pass runs after the debounce window (subject to the minimum pass gap) instead of waiting for the next periodic rescan

#### Scenario: Appended messages sync on the next pass

- **WHEN** new messages are appended to a previously synced session file
- **THEN** the next sync pass detects the change (size/mtime differ), re-delivers the session, and the appended messages become present in the archive (existing ones deduped)

#### Scenario: Rewritten session re-syncs without duplicates

- **WHEN** a database-backed or patch-log session is rewritten with additional messages
- **THEN** the daemon re-parses and re-delivers the session, and only the new messages are stored (already-stored ones are deduped by message key at the hub)

#### Scenario: Safety-net rescan catches a change

- **WHEN** a session file changes and the periodic rescan interval elapses
- **THEN** the rescan detects the divergence from the checkpoint and delivers the missing messages

### Requirement: Crash-safe local checkpoint

The daemon SHALL persist a local checkpoint that records, per session file, at least its size, mtime, message count, and last-synced timestamp. The checkpoint MUST be durable across restarts (written atomically) so the daemon never re-sends already-acknowledged data unnecessarily and never loses track of un-synced data.

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

### Requirement: Chronically failing sessions back off instead of retrying at full cost

A session whose delivery fails on every pass MUST NOT cost the full retry ladder on every pass. The daemon SHALL record a per-file consecutive-failure streak in the checkpoint; after a small grace window of immediate retries it SHALL defer that session on an exponentially widening schedule up to a daily ceiling, count the deferral separately from errors, and log the transition into backoff. Deferral is never abandonment: the session is still retried on the widened schedule, any change to the file's size or mtime resets the streak and forces an immediate attempt, and a successful delivery clears the streak entirely.

The daemon SHALL also make a failing delivery diagnosable: every retry warning and final error MUST name the session, its payload size, and the classified transport cause (timeout / connect / body / decode) with the error's full `source` chain — a bare "error sending request" is not actionable. The per-request timeout SHALL scale with payload size rather than being flat, so that the largest sessions are not the ones that permanently fail.

#### Scenario: A session failing every pass stops dominating the pass

- **WHEN** the same session fails delivery on more consecutive passes than the grace window allows
- **THEN** subsequent passes skip it without parsing or attempting delivery until its backoff window elapses, and it is reported as deferred rather than re-erroring

#### Scenario: Editing a deferred session forces an immediate retry

- **WHEN** a deferred session's file changes size or mtime
- **THEN** the next pass attempts delivery immediately and the failure streak restarts from zero

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
