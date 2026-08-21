# journal-entries (delta)

> Archive **after** `journal-day-bucketing`: this MODIFIED text is written on top
> of that change's version of the same requirement, not on the current main spec.

## MODIFIED Requirements

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
picks it up, whichever tick that is. The interval bounds staleness in ticks, not in
hours, and those are different units on any real host: `StartInterval` re-arms at the
previous run's **exit** rather than on a fixed grid, so a day holds at most
`86400 / (interval + run duration)` ticks even fully awake, and a host that sleeps skips
intervals outright and coalesces the missed ones into a single, delayed catch-up. No
requirement here, and no operational prediction drawn from it, may assume a tick has
occurred merely because an interval has elapsed. A tick that finds nothing
pending SHALL exit without making any LLM call. Hub HTTP calls (pending query, message
fetches, entry POST) SHALL be retried on transient failures (connection errors, 5xx)
with a bounded backoff before the tick gives up; a failed tick recovers at the next
tick, never at +24h.

Because elapsed time is not evidence that a tick ran, the distiller SHALL record each
tick with the hub: once per invocation, immediately after the pending query succeeds
and before any LLM call, stating its mode (`forward` or `backfill`) and the number of
groups it found pending. A `--dry-run` SHALL record nothing, consistent with its
contract of never writing. The record asserts only that a distiller reached the hub and
obtained a work list — whether that work then got done is what the pending list and the
staleness check already answer.

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

> The scenario name is kept from the original spec because a MODIFIED block may not
> drop or rename a scenario the main spec still carries. **"Within an hour" is the
> part this change corrects** — read the THEN.

- **WHEN** a logical day closes at 04:00 UTC, the machine's local timezone is in
  either its standard or DST offset, **and the host stays awake across the
  following interval**
- **THEN** the first tick after the close queries pending, sees the closed day's
  groups, and distills them — no tick's local wall-clock position can put it
  systematically before the close, which is the property the interval schedule
  exists for. That tick is **not** guaranteed to fall within an hour of the close
  even under the awake precondition, because the interval is measured from the
  previous run's exit; and the awake precondition is not decoration — see the next
  scenario for what the same schedule guarantees when it does not hold.

#### Scenario: A sleeping host delays the tick, and elapsed hours do not prove one ran

- **WHEN** the host sleeps across several `StartInterval` periods
- **THEN** the missed intervals are coalesced into one catch-up tick rather than
  replayed, so closed days stay pending until the host is back up and that tick has
  run, and the elapsed wall-clock time is not evidence of any tick having run

#### Scenario: Transient hub failure costs one tick at most

- **WHEN** the pending query (or a message fetch / entry POST) fails with a
  connection error or 5xx during a tick, and retries within the tick also fail
- **THEN** the tick exits non-zero without crash-looping, and the next tick retries
  the same still-pending work, so recovery latency is bounded by ticks

#### Scenario: Idle tick is free

- **WHEN** a tick runs and the pending list is empty
- **THEN** the distiller exits 0 having made no LLM call

#### Scenario: An idle tick is still recorded

- **WHEN** a tick runs, finds nothing pending, and exits without an LLM call
- **THEN** it has recorded the tick with the hub, so the absence of work is
  distinguishable from the absence of a distiller

#### Scenario: A dry run is not a tick

- **WHEN** the distiller is invoked with `--dry-run`
- **THEN** no tick is recorded, matching the flag's contract of writing nothing

#### Scenario: Late data heals within a tick

- **WHEN** a machine ingests sessions for an already-distilled day (dirtying
  its group) at an arbitrary time of day
- **THEN** the group is re-distilled by the next tick, not the next calendar
  day

## ADDED Requirements

### Requirement: Distiller tick record endpoint

The hub SHALL expose `POST /v1/journal/ticks`, authenticated with a machine token
(the same model as the entry upsert), accepting a tick record with a `mode` of
`forward` or `backfill` and a non-negative `groups_pending` count, and storing it
with a server-assigned timestamp. An unrecognized mode or a negative count SHALL be
rejected with a 400 and no write. Records SHALL be retained for a bounded window
sufficient to answer recent-tick questions (30 days), older ones pruned automatically
rather than by a separate job.

The endpoint exists because an idle tick is otherwise invisible to the hub: its only
interaction is a read, so nothing distinguishes a distiller ticking hourly into an
empty work list from one that has not run at all. Tick liveness MUST NOT be inferred
from `journal_entries.generated_at`, which moves only when a tick both had work and
succeeded at it.

#### Scenario: A tick is recorded

- **WHEN** a distiller POSTs a tick record with a valid mode and pending count
- **THEN** the hub stores it with its own timestamp and the record is reflected in
  `GET /v1/healthz/journal`

#### Scenario: An invalid tick record is rejected

- **WHEN** the posted mode is not `forward` or `backfill`, or `groups_pending` is
  negative
- **THEN** the hub returns 400 naming the offending field and writes nothing

#### Scenario: Recording a tick requires a machine token

- **WHEN** the tick record is posted without a valid machine token
- **THEN** the hub rejects it unauthenticated, like the entry upsert
