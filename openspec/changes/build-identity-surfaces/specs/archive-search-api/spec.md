# archive-search-api (delta)

## MODIFIED Requirements

### Requirement: Health endpoint

The hub SHALL expose an unauthenticated `GET /v1/healthz` endpoint that reports whether
the service and its database connection are operational, suitable for liveness checks
by the daemon and for deployment monitoring.

The response SHALL carry the hub's release version as `version`, an exact semver
string equal to the version the release was cut at (the single source of truth that
`package.json` holds and the build inherits). The field exists so that a deploy is
proven by reading the running service rather than by inferring the build from which
routes answer — a probe whose form has already been wrong twice and is vacuous for any
release that adds no route.

Consumers comparing the field MUST compare exact semver, never a prefix or substring
(`0.18.1` is a prefix of `0.18.10`), and a monitor that pins the field to one value
MUST move that pin with the binary — a pinned assertion that nobody widens before the
next swap turns every release into a false alarm.

#### Scenario: Healthz reflects database connectivity

- **WHEN** the hub can reach Postgres
- **THEN** `GET /v1/healthz` responds `200` with a healthy status

#### Scenario: Healthz names the running build

- **WHEN** a hub built at release `X.Y.Z` is polled
- **THEN** `GET /v1/healthz` carries `"version": "X.Y.Z"`, whether or not the
  database is reachable

#### Scenario: A swap is proven by the version, not by a route

- **WHEN** the hub binary is replaced by one built at a newer release
- **THEN** the `version` field changes to the newer release on the first poll after
  the swap, and a release that adds no new route is proven the same way
