# journal-entries — Delta

## MODIFIED Requirements

### Requirement: Distiller job

A distiller job (standalone script, deployed as a launchd agent on the hub
machine) SHALL drain the pending work list: for each group it fetches the
sessions' archived messages from the hub, generates the entry with a single
LLM call (single-turn; default model Haiku-tier, configurable), validates the
result against the entry schema, and POSTs it back. The distiller MUST be
idempotent and resumable (state lives in the hub, not the script), MUST run
non-interactively under launchd conforming to the house launchd-resilience
contract (never prompt headless; bao-first token resolution; degrade, don't
crash-loop), and MUST NOT automatically process groups older than its
configured forward horizon — historical backfill happens only via an explicit
`--backfill` invocation with date/limit bounds, newest-first. A `--dry-run`
mode SHALL produce and validate an entry without writing.

The job SHALL be scheduled as frequent idempotent ticks (fixed interval ≤1h,
`StartInterval`, plus run-at-load) rather than a calendar-time daily run, so
that no tick's wall-clock position relative to the 04:00 UTC logical-day
close — under any DST offset — determines whether a closed day is seen: some
tick within the hour after close MUST pick it up. A tick that finds nothing
pending SHALL exit without making any LLM call. Hub HTTP calls (pending
query, message fetches, entry POST) SHALL be retried on transient failures
(connection errors, 5xx) with a bounded backoff before the tick gives up;
a failed tick recovers at the next interval, never at +24h.

#### Scenario: Normal run drains pending

- **WHEN** the distiller runs and the pending list has closed groups within
  the forward horizon
- **THEN** each group results in exactly one upserted entry or skip row, and a
  re-run with no new data finds nothing pending

#### Scenario: Malformed LLM output is not stored

- **WHEN** the LLM returns output that fails entry-schema validation
- **THEN** the distiller does not POST it, logs the failure, and leaves the
  group pending

#### Scenario: Backfill is bounded and resumable

- **WHEN** `--backfill --from 2026-05-01 --limit 50` is invoked twice
- **THEN** the first run distills the 50 newest pending groups since that
  date, and the second run continues with the next 50 (no duplicates, no
  gaps)

#### Scenario: Closed day is distilled within an hour regardless of DST

- **WHEN** a logical day closes at 04:00 UTC and the machine's local timezone
  is in either its standard or DST offset
- **THEN** a tick within the following hour queries pending, sees the closed
  day's groups, and distills them — entries for yesterday exist by ~05:00 UTC

#### Scenario: Transient hub failure costs one tick at most

- **WHEN** the pending query (or a message fetch / entry POST) fails with a
  connection error or 5xx during a tick, and retries within the tick also fail
- **THEN** the tick exits non-zero without crash-looping, and the next
  interval tick retries the same still-pending work, so recovery latency is
  bounded by the tick interval

#### Scenario: Idle tick is free

- **WHEN** a tick runs and the pending list is empty
- **THEN** the distiller exits 0 having made no LLM call

#### Scenario: Late data heals within a tick

- **WHEN** a machine ingests sessions for an already-distilled day (dirtying
  its group) at an arbitrary time of day
- **THEN** the group is re-distilled by the next tick, not the next calendar
  day
