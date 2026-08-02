## ADDED Requirements

### Requirement: Snapshot enumeration across local and foreign stores

The backfill tool SHALL enumerate Time Machine backups by inspecting the backup store's APFS volume for `com.apple.TimeMachine.*` snapshots, working identically for the machine's active Time Machine destination (the default) and for any foreign backup store supplied by path (e.g. a retired machine's Time Machine disk attached to this Mac). A listing mode SHALL report, per snapshot, whether a Claude history directory is present, and MUST warn when the gap between consecutive snapshots exceeds 30 days (history last touched in the uncovered head of that gap is unrecoverable).

#### Scenario: Listing the local destination

- **WHEN** the operator runs the tool in list mode with no store argument
- **THEN** it resolves the active Time Machine destination, lists its snapshots with dates, and marks which contain Claude history

#### Scenario: Listing a foreign store

- **WHEN** the operator passes the mounted path of another machine's Time Machine disk
- **THEN** snapshots are enumerated from that store without requiring it to be the active destination

#### Scenario: Coverage gap warning

- **WHEN** two consecutive snapshots are more than 30 days apart
- **THEN** the listing flags the gap and the date range whose history is unrecoverable

### Requirement: Read-only snapshot mounting with guaranteed cleanup

The tool SHALL mount each Time Machine snapshot read-only for the duration of its ingestion and MUST unmount it afterwards, including when ingestion fails or the tool is interrupted. Mounts MUST NOT be left behind on exit.

#### Scenario: Unmount on success and on failure

- **WHEN** a snapshot's ingestion completes, fails, or is interrupted
- **THEN** the snapshot is unmounted before the tool exits

### Requirement: Isolated staging that never touches live state

For each snapshot the tool SHALL stage an isolated fake home directory whose `.claude` entry references the Claude history inside the mounted snapshot, and run the ingest against that fake home. The tool MUST NOT read, write, or otherwise disturb the live `~/.config/claude`, the live daemon's state directory, or the live daemon process. Root selection inside the backup MUST prefer a real `.config/claude` directory and MUST NOT follow symlinks when choosing the root (in backups of this fleet, `.claude` is an absolute symlink that resolves to the live filesystem of the machine performing the restore).

#### Scenario: In-backup absolute symlink is not followed

- **WHEN** a snapshot's home contains `.claude` as an absolute symlink and `.config/claude` as a real directory
- **THEN** the tool stages from the real `.config/claude` directory inside the snapshot and never reads through the symlink

#### Scenario: Stock layout fallback

- **WHEN** a snapshot's home contains a real `.claude` directory and no `.config/claude`
- **THEN** the tool stages from that `.claude` directory

#### Scenario: Live state untouched

- **WHEN** a backfill run completes
- **THEN** the live daemon's checkpoint and the live Claude config directory are byte-identical to before the run

### Requirement: Ingestion with per-source-machine attribution

The tool SHALL ingest staged history into the hub by running the sync daemon in single-pass mode with a temporary configuration derived from the live daemon configuration (hub URL and token read from `~/.config/cchv/daemon.toml`, never hardcoded). Each source machine label SHALL have its own persistent backfill state directory so its machine id is stable across runs and snapshots: for the local machine the tool MUST reuse the live daemon's machine id, and for a foreign machine label it MUST use (creating on first run) a dedicated machine id plus a hostname override matching the label. The tool MUST verify hub reachability before mounting anything.

#### Scenario: Local backfill merges with existing machine provenance

- **WHEN** the operator backfills the local machine's own backups
- **THEN** ingested sessions carry the same machine id and hostname as the live daemon's ingests

#### Scenario: Foreign backfill attributes to the source machine

- **WHEN** the operator backfills an attached ac-mbp Time Machine disk with the machine label `ac-mbp`
- **THEN** ingested sessions carry a stable ac-mbp machine id and hostname `ac-mbp`, distinct from the host machine's identity, and re-running later reuses the same id

#### Scenario: Re-ingestion is idempotent

- **WHEN** the same snapshot (or overlapping snapshots) are ingested more than once
- **THEN** the archive contains no duplicate messages

#### Scenario: Unreachable hub aborts early

- **WHEN** the hub health endpoint is unreachable
- **THEN** the tool exits with an error before mounting any snapshot

### Requirement: Operator runbook

The repository SHALL contain a runbook documenting the end-to-end recovery procedure such that a future session on any machine of the fleet can execute it without re-deriving the mechanics. The runbook MUST cover: prerequisites (daemon binary, hub reachability, unsandboxed mount permission), per-machine usage against the machine's own backups, foreign-disk recovery (the ac-mbp case) including attribution, the coverage-window math with known gaps, and the in-backup symlink trap.

#### Scenario: Runbook enables recovery from a fresh session

- **WHEN** a future session on any fleet machine follows the runbook
- **THEN** it can list that machine's snapshots and ingest a chosen range into the hub using documented commands only
