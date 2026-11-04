## ADDED Requirements

### Requirement: Excluded paths never page

The journal health verdict SHALL ignore every pending group whose `project_path` matches the hub's journal exclusion patterns: the built-in OS temp roots plus `[journal] exclude_paths`. It SHALL use the same predicate as `GET /v1/journal/pending`.

#### Scenario: A temp-path group stays out of the verdict
- **WHEN** the only stale in-window group is under `/private/tmp/`
- **THEN** the check answers 200 with `status: "ok"`

#### Scenario: A configured scratch path stays out of the verdict
- **WHEN** `exclude_paths` holds `~/.local/state/scratch/*`, `home` is set, and the only stale group lies under it
- **THEN** the check answers 200 with `status: "ok"`

#### Scenario: Health still agrees with pending under exclusions
- **WHEN** an exclusion list is configured
- **THEN** the set of groups health evaluates equals the set `pending` returns for the same window

### Requirement: Client groups page only when they can be drained

The body SHALL report `client_undistilled` (the in-window pending client groups) and `client_groups_page`, and SHALL mark each listed group with `client`. A stale client group SHALL count toward `stale` only when client scope is not enforcing or at least one client reader is configured.

#### Scenario: Undrainable client group is reported, not red
- **WHEN** client scope enforces, no client reader is configured, and the only stale group is a client group
- **THEN** the check answers 200 with `status: "ok"`, `client_undistilled: 1` and `client_groups_page: false`

#### Scenario: Drainable client group pages
- **WHEN** client scope enforces with one client reader configured and a client group is stale
- **THEN** the check answers 503 with `status: "stale"`

#### Scenario: Observe mode counts client groups
- **WHEN** client scope does not enforce and a client group is stale
- **THEN** the check answers 503 with `status: "stale"`

### Requirement: A deliberate pause reads as paused

When the newest distiller tick has mode `paused`, the check SHALL answer 200 with `status: "paused"`, `paused_reason` and `paused_since`, whatever the backlog or tick age. A later `forward` or `backfill` tick SHALL end the pause.

#### Scenario: Pause outranks stale and no_tick
- **WHEN** the newest tick is `paused` with a reason, a group is stale, and `max_tick_age_secs` is exceeded
- **THEN** the check answers 200 with `status: "paused"` and the reason

#### Scenario: The next run ends the pause
- **WHEN** a `forward` tick is recorded after the `paused` one and a group is stale
- **THEN** the check answers 503 with `status: "stale"`
