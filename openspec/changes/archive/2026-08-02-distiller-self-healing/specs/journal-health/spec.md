# journal-health — Delta

## ADDED Requirements

### Requirement: Journal staleness health endpoint

The hub SHALL expose an unauthenticated `GET /v1/healthz/journal` endpoint,
consumable by HTTP monitors that read only status code and body (Gatus). The
endpoint SHALL derive pending groups for closed logical days using the same
logical-day fold and pending semantics as `GET /v1/journal/pending`, and for
each group compute its latest data arrival (`max(messages.created_at)` over
the group's sessions).

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

#### Scenario: Undrained closed day pages

- **WHEN** a closed logical day has pending groups whose latest data arrived
  more than `grace_secs` ago and no distiller has drained them
- **THEN** the endpoint returns 503 with status `"stale"` and the offending
  groups in the body

#### Scenario: Freshly dirtied day stays green within grace

- **WHEN** a late-waking machine ingests sessions that re-pend an
  already-distilled day, and the data arrived less than `grace_secs` ago
- **THEN** the endpoint returns 200 with status `"ok"` (the hourly tick still
  has time to drain it)

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

#### Scenario: Invalid parameters

- **WHEN** `?grace_secs=abc`, `?grace_secs=0`, or `?within_days=-1` is supplied
- **THEN** the endpoint returns 400 with a message naming the offending
  parameter
