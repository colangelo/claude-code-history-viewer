# journal-entries (delta)

## MODIFIED Requirements

### Requirement: Logical-day fold

A session SHALL belong to the entry group of **every** logical date on which it has at
least one archived message — not to the single date it began. A message's logical date
is its timestamp shifted by a configurable `day_start_hour` (default 04:00): a message
before `day_start_hour` counts toward the previous calendar day, so late-night work
lands in the day it belongs to. A group's session set is therefore the set of sessions
carrying at least one message that day, and a session that spans midnight is a member
of both days.

The fold MUST be computed from one shared definition, so that the pending work list,
the write endpoint's provenance check and the journal-staleness check cannot disagree
about which sessions belong to a day.

#### Scenario: Late-night session folds to previous day

- **WHEN** a message's timestamp is 02:30 local time on 2026-07-12 and
  `day_start_hour` is 04:00
- **THEN** that message counts toward the 2026-07-11 entry group

#### Scenario: A session spanning midnight belongs to both days

- **WHEN** a session's first message is on 2026-08-19 at 16:05 and its last is on
  2026-08-20 at 22:43, with messages on both logical days
- **THEN** the session is a member of both the 2026-08-19 and the 2026-08-20 entry
  groups for its project

#### Scenario: A day with no session start is still a group

- **WHEN** every message on a logical date belongs to sessions that started on an
  earlier date
- **THEN** that date still forms an entry group for its project, containing those
  sessions

#### Scenario: An idle day inside a session's span is not a group

- **WHEN** a session's first message is on 2026-08-01, its last on 2026-08-05, and it
  has no messages at all on 2026-08-03
- **THEN** 2026-08-03 is not an entry group for that session

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

### Requirement: Journal write endpoint

The hub SHALL expose `POST /v1/journal/entries` authenticated by machine token (as
ingest is), upserting by `(entry_date, project_path)`: re-distilling a dirty group
replaces the previous entry and refreshes its generation watermark. The endpoint MUST
validate entry payloads (status is `entry` or `skip`; `entry` payloads carry headline,
summary, 3–8 topics, and a non-empty model) and enforce **exact provenance**: every
referenced session id MUST have at least one archived message inside the posted
(entry_date, project_path) group under the logical-day fold, and the set MUST cover
every such session — mismatched or partial provenance is rejected. Stored session ids
SHALL be canonical (sorted, deduplicated) so that provenance can be compared by value.
Invalid payloads are rejected with a `4xx` and a reason, without partial writes.

#### Scenario: Upsert replaces a dirty entry

- **WHEN** an entry exists for a group and a new distillation is POSTed for the same
  (entry_date, project_path)
- **THEN** the stored entry reflects the new content and `generated_at`, and no
  duplicate row exists

#### Scenario: A midnight-spanning session is accepted for both days

- **WHEN** an entry for 2026-08-19 and an entry for 2026-08-20 are each POSTed
  carrying the id of a session with messages on both days
- **THEN** both are accepted

#### Scenario: A session with no message that day is rejected

- **WHEN** an entry payload names a session that exists in the project but has no
  message inside the posted logical day
- **THEN** the hub responds `4xx` and stores nothing

#### Scenario: Invalid payload is rejected atomically

- **WHEN** an `entry`-status payload is POSTed with zero topics or a nonexistent
  session id
- **THEN** the hub responds `4xx` with a reason and stores nothing

### Requirement: Distiller job

A distiller job (standalone script, deployed as a launchd agent on the hub machine)
SHALL drain the pending work list: for each group it fetches **that group's logical
day** of the sessions' archived messages from the hub — not the sessions in their
entirety — generates the entry with a single LLM call (single-turn; default model
Haiku-tier, configurable), validates the result against the entry schema, and POSTs it
back. When a group's windowed transcript contains no usable content the distiller
SHALL post a `skip` row without making an LLM call. The distiller MUST be idempotent
and resumable (state lives in the hub, not the script), MUST run non-interactively
under launchd conforming to the house launchd-resilience contract (never prompt
headless; bao-first token resolution; degrade, don't crash-loop), and MUST NOT
automatically process groups older than its configured forward horizon — historical
backfill happens only via an explicit `--backfill` invocation with date/limit bounds,
newest-first. A `--dry-run` mode SHALL produce and validate an entry without writing.

That forward horizon SHALL be measured from the **current logical day** — the same
`day_start_hour` fold the hub applies, in UTC — and not from the machine's local
calendar date. The two windows this bounds are the tick's own work list and the one
`GET /v1/healthz/journal` evaluates, and they MUST be the same window: anchoring them
differently makes the check page for groups no forward tick will ever pick up, every
night between 00:00 UTC and `day_start_hour`.

The job SHALL be scheduled as frequent idempotent ticks (fixed interval ≤1h,
`StartInterval`, plus run-at-load) rather than a calendar-time daily run, so that no
tick's wall-clock position relative to the 04:00 UTC logical-day close — under any DST
offset — determines whether a closed day is seen: the **first tick after the close**
MUST pick it up, whichever tick that is. (Not "some tick within the hour": the interval
re-arms at the previous run's exit, and a sleeping host skips intervals outright.
`distiller-tick-observability`, which archives after this change, restates this
requirement with the measurement behind it.) A tick that finds nothing pending SHALL
exit without making any LLM call. Hub HTTP calls (pending query, message fetches, entry POST) SHALL be retried on
transient failures (connection errors, 5xx) with a bounded backoff before the tick
gives up; a failed tick recovers at the next interval, never at +24h.

#### Scenario: Normal run drains pending

- **WHEN** the distiller runs and the pending list has closed groups within the forward
  horizon
- **THEN** each group results in exactly one upserted entry or skip row, and a re-run
  with no new data finds nothing pending

#### Scenario: The tick's horizon and the health check's window are the same window

- **WHEN** a forward tick runs at 00:46 UTC on 2026-08-21 with `--horizon-days 7`
  and `day_start_hour` is 04:00 (so the current logical day is still 2026-08-20)
- **THEN** its date lower bound is 2026-08-13 — the same bound
  `GET /v1/healthz/journal?within_days=7` evaluates — and no group is counted stale
  that the tick excludes

#### Scenario: Only the group's day reaches the prompt

- **WHEN** a group's session ran from 2026-08-19 into 2026-08-20 and the 2026-08-19
  group is distilled
- **THEN** the transcript passed to the LLM contains only that session's 2026-08-19
  messages, and the 2026-08-20 messages appear in the 2026-08-20 group instead

#### Scenario: An empty windowed transcript costs no LLM call

- **WHEN** every message in a group's day is filtered out of the transcript
- **THEN** the distiller posts a `skip` row for the group and makes no LLM call

#### Scenario: Malformed LLM output is not stored

- **WHEN** the LLM returns output that fails entry-schema validation
- **THEN** the distiller does not POST it, logs the failure, and leaves the group
  pending

#### Scenario: Backfill is bounded and resumable

- **WHEN** `--backfill --from 2026-05-01 --limit 50` is invoked twice
- **THEN** the first run distills the 50 newest pending groups since that date, and the
  second run continues with the next 50 (no duplicates, no gaps)

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
