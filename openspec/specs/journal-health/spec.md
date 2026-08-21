# journal-health Specification

## Purpose

Making journal distillation staleness externally observable, so that a
distiller that has stopped draining closed logical days is caught by monitoring
rather than by a human noticing missing entries. Read-only and unauthenticated
so a plain HTTP monitor can consume it.

## Requirements

### Requirement: Journal staleness health endpoint

The hub SHALL expose an unauthenticated `GET /v1/healthz/journal` endpoint,
consumable by HTTP monitors that read only status code and body (Gatus). The
endpoint SHALL derive pending groups for closed logical days using the same
logical-day fold and pending semantics as `GET /v1/journal/pending` — including
membership by message date, so a session that spans midnight is evaluated under both
days, and including provenance drift as a pending condition — and for each group
compute its latest data arrival (`max(messages.created_at)` over the group's sessions).
The fold MUST come from the shared definition the pending endpoint uses, not a
re-derivation, so the two cannot disagree about which sessions belong to a day.

Evaluation SHALL be bounded to the distiller's forward horizon: only groups
whose `entry_date` is within `within_days` (query param; default 7, matching
the distiller's `--horizon-days`) of the current logical day are considered.
Groups older than the horizon are awaiting explicit `--backfill`, are never
auto-distilled, and MUST NOT flip the endpoint stale — the archive routinely
holds hundreds of such never-auto-distilled historical groups.

Among the in-window groups, one is stale when `now − latest_arrival` exceeds a
grace window (`grace_secs` query param; default 7200). When any in-window
group is stale the endpoint SHALL return 503 with status `"stale"`; otherwise
200 with status `"ok"`. The body SHALL list the evaluated (in-window pending)
groups (entry date, project path, latest arrival, stale flag) for
observability. A non-numeric or non-positive `grace_secs` or `within_days`
SHALL return 400 via the standard error path.

Because grace is measured from each group's `latest_arrival` and not from the
day close, a logical day that closes with groups whose data arrived more than
`grace_secs` earlier is stale the instant it closes. A day close can therefore
only add stale groups on net, never remove them: **only a distiller tick clears
this check, never the passage of time**. No wall-clock recovery time may be
inferred from a 503.

The response SHALL additionally report distiller-tick liveness alongside the
group list, so that "a backlog is draining" and "no distiller has run" are
distinguishable from one another rather than being the same 503: `last_tick_at`,
`last_tick_age_secs`, `last_tick_mode` and `last_tick_groups_pending` (all null
when no tick has ever been recorded), and `ticks_last_24h`.

The response SHALL also carry identity next to liveness: `hub_version` (the hub's own
release version, the same value `GET /v1/healthz` reports) and, from the most recent
recorded tick, `last_tick_distiller_version` and `last_tick_distiller_blob` — the
release version the distiller script was cut at and the git blob id of the file that
actually ran. Both are `null` when no tick has been recorded or when the last tick was
posted by a distiller that predates this field, which is itself the reading "an old
distiller is ticking". A release deploys in two halves through different hands — a
hub swap and a distiller reinstall — and this is the one read that says whether both
landed: a `hub_version` ahead of `last_tick_distiller_version` after a tick means the
distiller half is still the old copy. The blob id is the exact identity: it equals
`git rev-parse <rev>:scripts/cchv-distill.py` for whichever revision the installed copy
matches, so a reader with the repo can name the commit the copy came from — or show that
it matches none.

Tick age SHALL NOT contribute to the verdict unless a `max_tick_age_secs` query
param is supplied. Absent, tick liveness is reported and never alerts — the host
running the distiller sleeps many times a day, so any default threshold would be
an assumption about that host's wake schedule rather than a property of the
archive. When `max_tick_age_secs` is supplied and either no tick has ever been
recorded or `now − last_tick_at` exceeds it, the endpoint SHALL return 503 with
status `"no_tick"`, which takes precedence over `"stale"` — when both conditions
hold, the absent tick is the cause and the stale groups are its symptom. A
non-numeric or non-positive `max_tick_age_secs` SHALL return 400 via the standard
error path.

#### Scenario: Undrained closed day pages

- **WHEN** a closed logical day has pending groups whose latest data arrived
  more than `grace_secs` ago and no distiller has drained them
- **THEN** the endpoint returns 503 with status `"stale"` and the offending
  groups in the body

#### Scenario: Health agrees with pending about a midnight-spanning session

- **WHEN** a session has messages on two consecutive closed logical days
- **THEN** both days appear as groups in the health evaluation, matching the groups
  `GET /v1/journal/pending` reports for the same window

#### Scenario: Freshly dirtied day stays green within grace

- **WHEN** a late-waking machine ingests sessions that re-pend an
  already-distilled day, and the data arrived less than `grace_secs` ago
- **THEN** the endpoint returns 200 with status `"ok"` (the next tick still has
  time to drain it)

#### Scenario: A closing day admits groups that are already stale

- **WHEN** a logical day closes carrying pending groups whose latest data
  arrived more than `grace_secs` before the close
- **THEN** those groups are stale immediately, with no grace remaining, and the
  endpoint returns 503 — the close is not a recovery event

#### Scenario: Open day never pages

- **WHEN** the only groups with archived sessions and no journal entry belong
  to the still-open logical day
- **THEN** the endpoint returns 200 with status `"ok"`

#### Scenario: Old un-backfilled history never pages

- **WHEN** pending closed-day groups exist only for days older than
  `within_days` (never auto-distilled; awaiting explicit backfill), with data
  that arrived long ago
- **THEN** the endpoint returns 200 with status `"ok"` and does not list them

#### Scenario: Fully drained archive is healthy

- **WHEN** no pending groups exist for closed logical days
- **THEN** the endpoint returns 200 with status `"ok"` and an empty group list

#### Scenario: A recorded tick is visible to the monitor

- **WHEN** a distiller records a tick and the endpoint is then polled
- **THEN** the response carries that tick's timestamp, mode and pending count,
  a `last_tick_age_secs` derived from it, and a `ticks_last_24h` that counts it

#### Scenario: Tick age never alerts unless asked

- **WHEN** no tick has ever been recorded, or the last one is long past, and no
  `max_tick_age_secs` is supplied
- **THEN** the verdict is decided by stale groups alone, exactly as before

#### Scenario: An absent tick outranks a stale backlog

- **WHEN** `max_tick_age_secs` is supplied and exceeded while in-window groups
  are also stale
- **THEN** the endpoint returns 503 with status `"no_tick"`

#### Scenario: Both halves of a release are visible in one read

- **WHEN** the hub has been swapped to release `X.Y.Z` and a distiller cut at the same
  release has since ticked
- **THEN** the response carries `hub_version` `X.Y.Z`, `last_tick_distiller_version`
  `X.Y.Z`, and a 40-hex `last_tick_distiller_blob`

#### Scenario: A stale distiller copy is visible without touching the host

- **WHEN** the hub is at release `X.Y.Z` but the last tick was posted by a distiller
  whose script was cut at an earlier release
- **THEN** `last_tick_distiller_version` reports the earlier release and
  `last_tick_distiller_blob` the blob of the file that ran, and the verdict
  (`ok` / `stale` / `no_tick`) is unaffected — identity is reported, never alerted on

#### Scenario: A pre-identity distiller reads as null

- **WHEN** the last recorded tick carried no identity fields
- **THEN** `last_tick_distiller_version` and `last_tick_distiller_blob` are `null`
  while `last_tick_at` and the other tick fields are populated as before

#### Scenario: Invalid parameters

- **WHEN** `?grace_secs=abc`, `?grace_secs=0`, `?within_days=-1`, or
  `?max_tick_age_secs=0` is supplied
- **THEN** the endpoint returns 400 with a message naming the offending
  parameter
