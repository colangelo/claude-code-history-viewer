# journal-entries Specification

## Purpose

Distilled per-(date, project) journal entries generated from archived
sessions: high-quality retrieval units (headline, summary, topics,
open_questions, session provenance) folded across machines, with a
catch-up/dirty work-list contract for the distiller and read/write endpoints.
Ported from engineering-notebook's summarize layer (Apache-2.0); issue #12,
shipped cchv-v0.6.0.

## Requirements

### Requirement: Journal entry schema and storage

The hub SHALL store journal entries in a `journal_entries` table keyed
uniquely by `(entry_date, project_path)` — one entry per logical date per
project, folded **across machines**. Each entry with `status = 'entry'` MUST
carry: a one-line `headline`, a 2–5 sentence `summary`, 3–8 `topics`,
`open_questions` (zero or more dropped/unresolved threads), and the surrogate
ids of every archived session it distills (machine provenance is reachable
through those sessions). Each entry MUST record the `model` that generated it
and a `generated_at` timestamp. Entry text SHALL be indexed for Postgres
full-text search (its own tsvector + GIN index); the migration MUST apply
cleanly to an existing archive database without touching existing tables.

#### Scenario: One entry per date and project across machines

- **WHEN** sessions for the same project path on the same logical date exist
  from two different machines and the group is distilled
- **THEN** exactly one journal entry exists for that (date, project_path),
  and its session ids reference sessions from both machines

#### Scenario: Entry content is complete

- **WHEN** a group with substantive activity is distilled
- **THEN** the stored entry has a non-empty headline, a summary, between 3 and
  8 topics, and at least one session id, with `generated_at` set

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

### Requirement: Skip sentinel for non-substantive days

When the distiller judges a (date, project) group non-substantive, the hub
SHALL store a `status = 'skip'` row carrying the group key, session ids, and
`generated_at` — but no headline/summary/topics. Skip rows MUST act as
watermarks: a skipped group MUST NOT reappear in the pending work list unless
new session data arrives for it, and skip rows MUST NOT appear in journal
search results or browse listings unless explicitly requested.

#### Scenario: Skipped group is not re-attempted

- **WHEN** a group was marked `skip` and no new sessions have arrived for it
- **THEN** the pending work list does not include that group

#### Scenario: New data revives a skipped group

- **WHEN** a group was marked `skip` and a new session for that (date,
  project_path) is later ingested
- **THEN** the group reappears in the pending work list

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

### Requirement: Journal browse endpoint

The hub SHALL expose `GET /v1/journal/entries` (read-auth) listing entries
filterable by project and date range, newest-first, paginated, returning full
entry content including session ids so a caller can drill into the underlying
transcripts.

#### Scenario: Browse a project's journal

- **WHEN** an authenticated client lists journal entries filtered to one
  project over a date range
- **THEN** matching `entry`-status rows are returned newest-first with
  headline, summary, topics, open_questions, and session ids

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

The distiller SHALL announce its own identity as the first thing it logs on every
invocation, and SHALL carry the same identity on the tick record: the release version
its script was cut at, and the git blob id of the file that is actually running —
computed at start from the bytes of that file, not read from any side channel, so that
an installed copy and the repository's copy can be compared from the log alone. The
installed distiller is a copy, not a symlink, and that stays so: the working tree it
would point into is shared by a replicator, and a symlink would put a peer's in-flight
edit into production on the next tick with no deploy boundary. The identity line exists
because a copy that says nothing about itself let a stale build run for hours behind a
green `main` (2026-08-21); the remedy is not to make staleness impossible but to make it
visible where someone already looks.

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

#### Scenario: The first log line names the running copy

- **WHEN** the distiller starts, in any mode including `--dry-run`
- **THEN** before any hub call it logs its release version and the git blob id of its
  own file, and `git rev-parse <rev>:scripts/cchv-distill.py` for the revision the copy
  was installed from equals that blob id

#### Scenario: The tick record carries the same identity

- **WHEN** a tick is recorded
- **THEN** the record carries the version and blob id the log line announced, so the
  hub can report which copy ticked

#### Scenario: A dry run is not a tick

- **WHEN** the distiller is invoked with `--dry-run`
- **THEN** no tick is recorded, matching the flag's contract of writing nothing

#### Scenario: Late data heals within a tick

- **WHEN** a machine ingests sessions for an already-distilled day (dirtying
  its group) at an arbitrary time of day
- **THEN** the group is re-distilled by the next tick, not the next calendar
  day

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

The record MAY additionally carry `distiller_version` (a release version string) and
`distiller_blob` (a git blob id: exactly 40 lowercase hexadecimal characters). Both are
optional so that a distiller predating them keeps ticking unchanged; when absent they
are stored as null. When `distiller_blob` is present but not 40 lowercase hex, the
record SHALL be rejected with a 400 naming the field and no write — a malformed
identity is worse than none, because it reads as one. The most recent record's
identity fields SHALL be reported by `GET /v1/healthz/journal`.

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

#### Scenario: A tick with identity is stored with it

- **WHEN** a distiller POSTs a tick record carrying `distiller_version` and a 40-hex
  `distiller_blob`
- **THEN** the hub stores both and `GET /v1/healthz/journal` reports them as the last
  tick's identity

#### Scenario: A tick without identity is still a tick

- **WHEN** a distiller POSTs a tick record with neither identity field
- **THEN** the hub stores the tick with null identity and liveness is reported exactly
  as before

#### Scenario: A malformed blob id is rejected

- **WHEN** `distiller_blob` is present and is not exactly 40 lowercase hex characters
- **THEN** the hub returns 400 naming `distiller_blob` and writes nothing
### Requirement: Identity-scoped journal reads

`GET /v1/journal/entries` and journal search SHALL accept the
`identity:<key>` project filter (expansion to member + aliased paths,
`include_worktrees` honored), so a moved repo's journal timeline reads as one
stream. Entry storage stays keyed by `(entry_date, project_path)` — identity
is a read-time lens; the pending work-list contract, write endpoint, and
distiller are unchanged. When a logical day has entries under two member
paths (a mid-day move), both entries are returned and the client renders them
under one identity heading.

#### Scenario: Unified timeline across a move

- **WHEN** a repo moved homes on day D and journal entries exist for the old path (before D) and new path (after D)
- **THEN** `GET /v1/journal/entries?project=identity:<key>` returns the full timeline across both paths

#### Scenario: Distiller contract untouched

- **WHEN** the distiller polls `/v1/journal/pending` and posts entries after this change ships
- **THEN** its requests and the hub's responses are identical to pre-identity behavior
