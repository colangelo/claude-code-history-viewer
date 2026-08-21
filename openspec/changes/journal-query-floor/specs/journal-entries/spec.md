# journal-entries (delta)

## MODIFIED Requirements

### Requirement: Catch-up pending work list

The hub SHALL expose `GET /v1/journal/pending` (read-auth) returning the
(date, project_path) groups needing distillation, computed from data — not from a
schedule. A group is pending when it has archived sessions but no journal row,
**or** when **that day's** message data became visible after the entry was generated
(dirty) — judged by transaction visibility, not wall-clock comparison, so an ingest
still in flight when the entry was generated counts as dirty regardless of timestamp
interleaving, and replaying an already-archived batch (no new messages) does NOT dirty
a group. The visibility test MUST be scoped to the messages of the group's own logical
day: a session that spans several days belongs to all of them, so testing at session
granularity lets one still-running session hold every day it has ever touched
permanently dirty — **or** when the stored entry's session ids differ from the group's computed
session set (provenance drift). Each pending group SHALL carry an `as_of` generation
marker (a database snapshot taken before the caller reads any transcript) that the
distiller echoes back in its entry POST, anchoring dirty-detection to the moment the
group was read. The endpoint MUST support bounding parameters (at minimum a date lower
bound and a result limit) so callers can take bounded, resumable chunks; results SHALL
be ordered newest-first. Groups whose logical date is not yet closed (today, per
`day_start_hour`) MUST NOT be listed.

The date lower bound MUST be applied to the underlying message scan, not only to the
computed group key, so a bounded call does not read the whole archive.

#### Scenario: Missing entry is pending

- **WHEN** archived sessions exist for a closed (date, project_path) group with no
  journal row
- **THEN** `/v1/journal/pending` lists that group

#### Scenario: Late-arriving session dirties an existing entry

- **WHEN** an entry was generated at T and a session for its group is ingested at T+1
  (e.g. a machine syncing days of backlog)
- **THEN** the group reappears in the pending list until re-distilled

#### Scenario: A still-running session does not dirty its own frozen days

- **WHEN** a session has messages on two logical days, both are distilled, and the
  session then gains a new message on the later day only
- **THEN** the later day is pending and the earlier day is NOT

#### Scenario: Provenance drift dirties an existing entry

- **WHEN** a stored entry's `session_ids` are not exactly the session set the group
  computes today
- **THEN** the group is listed as pending, and after re-distillation with the correct
  set it is not listed again

#### Scenario: Downtime only delays, never drops

- **WHEN** the distiller does not run for N days
- **THEN** all groups from those days are still pending on its next run

The pending list runs the same logical-day fold as the staleness check and is subject to the
same floor: computing it SHALL NOT require reading message payloads, and the access path that
makes this possible MUST NOT change which groups are returned — a day of nothing but agent
state records still appears, and still earns its `skip`.

#### Scenario: Pending is computed without reading message payloads

- **WHEN** the pending work list is computed for its window
- **THEN** it obtains each message's timestamp, session and arrival time without reading that
  message's content or raw payload, and returns the same groups it would have returned before
