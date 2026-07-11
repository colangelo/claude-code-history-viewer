# journal-entries Specification (delta)

## ADDED Requirements

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

A session SHALL be assigned to the logical date of its first message shifted
by a configurable `day_start_hour` (default 04:00): sessions starting before
`day_start_hour` local time count toward the previous calendar day, so
late-night work lands in the day it belongs to.

#### Scenario: Late-night session folds to previous day

- **WHEN** a session's first message is at 02:30 local time on 2026-07-12 and
  `day_start_hour` is 04:00
- **THEN** that session belongs to the 2026-07-11 entry group

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
(date, project_path) groups needing distillation, computed from data — not
from a schedule. A group is pending when it has archived sessions but no
journal row, **or** when session data for it became visible after the entry
was generated (dirty) — judged by transaction visibility, not wall-clock
comparison, so an ingest still in flight when the entry was generated counts
as dirty regardless of timestamp interleaving, and replaying an
already-archived batch (no new messages) does NOT dirty a group. Each pending
group SHALL carry an `as_of` generation marker (a database snapshot taken
before the caller reads any transcript) that the distiller echoes back in its
entry POST, anchoring dirty-detection to the moment the group was read. The
endpoint MUST support bounding parameters (at minimum a date lower bound and
a result limit) so callers can take bounded, resumable chunks; results SHALL
be ordered newest-first. Groups whose logical date is not yet closed (today,
per `day_start_hour`) MUST NOT be listed.

#### Scenario: Missing entry is pending

- **WHEN** archived sessions exist for a closed (date, project_path) group
  with no journal row
- **THEN** `/v1/journal/pending` lists that group

#### Scenario: Late-arriving session dirties an existing entry

- **WHEN** an entry was generated at T and a session for its group is ingested
  at T+1 (e.g. a machine syncing days of backlog)
- **THEN** the group reappears in the pending list until re-distilled

#### Scenario: Downtime only delays, never drops

- **WHEN** the distiller does not run for N days
- **THEN** all groups from those days are still pending on its next run

### Requirement: Journal write endpoint

The hub SHALL expose `POST /v1/journal/entries` authenticated by machine
token (as ingest is), upserting by `(entry_date, project_path)`: re-distilling
a dirty group replaces the previous entry and refreshes its generation
watermark. The endpoint MUST validate entry payloads (status is `entry` or
`skip`; `entry` payloads carry headline, summary, 3–8 topics, and a non-empty
model) and enforce **exact provenance**: every referenced session id MUST
belong to the posted (entry_date, project_path) group under the logical-day
fold, and the set MUST cover every archived session in the group — mismatched
or partial provenance is rejected. Invalid payloads are rejected with a `4xx`
and a reason, without partial writes.

#### Scenario: Upsert replaces a dirty entry

- **WHEN** an entry exists for a group and a new distillation is POSTed for
  the same (entry_date, project_path)
- **THEN** the stored entry reflects the new content and `generated_at`, and
  no duplicate row exists

#### Scenario: Invalid payload is rejected atomically

- **WHEN** an `entry`-status payload is POSTed with zero topics or a
  nonexistent session id
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
