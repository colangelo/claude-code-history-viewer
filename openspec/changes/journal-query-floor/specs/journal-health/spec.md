# journal-health (delta)

> Rebuilt on top of `build-identity-surfaces` (archived
> 2026-08-21): a MODIFIED block replaces the whole requirement, so this text must carry
> that change's identity fields and scenarios as well as its own additions.

## MODIFIED Requirements

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

The endpoint is monitor-polled, so its cost is part of its contract. Evaluating it SHALL
NOT require reading message payloads: the fold it runs needs only each message's timestamp,
session and arrival time, and it SHALL obtain those three without reading the content or raw
payload of the messages they belong to, and without spilling its aggregate to disk. Stated
this way — a bound on what the check is allowed to touch — rather than as a wall-clock
number, which would be a property of whichever machine happened to run it.

**AMENDED 2026-08-23, and the previous wording is worth recording because it was wrong in an
instructive way.** This sentence used to require "an access path that supplies those three
*without visiting the rows they belong to*" — which is a covering index and nothing else. It
therefore mandated a mechanism, while the scenario below that actually tests it asks only for
"without reading that message's content or raw payload", which a sequential scan already
satisfies (`content` is TOASTed and is never detoasted by a fold that does not select it).
Prose and scenario disagreed, and only the prose demanded the index. Measurement then showed
the mandated mechanism was not worth its price: against `work_mem` sized past the spill knee
the covering index buys ~5 % of the endpoint for ~400 MB, a production DDL and permanent write
amplification (#36 comments 7505/7511). The bound is now stated as the two things measured to
matter — do not read payloads, do not spill — which is what the failure this requirement
prevents was actually made of. The failure this prevents is concrete: reading the
payload columns to answer a question that does not use them means ~1.3 GB of heap for a
7-day window, which has already exceeded a proxy timeout and answered 502, and a health
check that flaps is worse than one that is slow because the flap is what gets investigated.

That access path MUST NOT change which groups the fold sees. In particular it MUST NOT be
restricted to messages carrying conversation content: a logical day composed entirely of
agent state records is still a day the distiller must consider and dispose of, and making
it invisible here would silently withdraw its `skip` row.

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

#### Scenario: The check does not read what it does not use

- **WHEN** the staleness evaluation runs over its window
- **THEN** it obtains each message's timestamp, session and arrival time without reading
  that message's content or raw payload

#### Scenario: A day of only state records is still evaluated

- **WHEN** a closed logical day's messages are all agent state records with no conversation
  content
- **THEN** that day is evaluated exactly as any other — the access path does not hide it

#### Scenario: Invalid parameters

- **WHEN** `?grace_secs=abc`, `?grace_secs=0`, `?within_days=-1`, or
  `?max_tick_age_secs=0` is supplied
- **THEN** the endpoint returns 400 with a message naming the offending
  parameter
