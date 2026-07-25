# hub-credential-resilience Specification

## Purpose
TBD - created by archiving change hub-db-auth-failfast. Update Purpose after archive.
## Requirements
### Requirement: Hub exits non-zero on a sustained database authentication failure

The hub SHALL terminate its own process with a non-zero exit status when Postgres
rejects authentication on a sustained basis. "Sustained" means a configured number of
**consecutive** authentication failures observed by the credential watchdog; a single
failure MUST NOT terminate the process.

The hub MUST NOT attempt to re-resolve or renew the credential in-process. Recovery is
delegated entirely to the process supervisor.

#### Scenario: Credential rotated underneath a running hub

- **WHEN** the database password is rotated while the hub is running
- **AND** the watchdog observes the configured number of consecutive authentication
  failures
- **THEN** the hub logs an error identifying a rotated or invalid credential as the cause
- **AND** the process exits with a non-zero status

#### Scenario: A single authentication failure is tolerated

- **WHEN** the watchdog observes one authentication failure
- **AND** the strike limit is greater than one
- **THEN** the hub continues running and does not exit

#### Scenario: The credential is never written to a log

- **WHEN** the watchdog logs any probe outcome, including a failure
- **THEN** the log record MUST NOT contain the database URL or the password

### Requirement: Only authentication failures count toward termination

The watchdog SHALL count **only** a Postgres authentication rejection (SQLSTATE
`28P01`, `invalid_password`) toward the strike total. Every other outcome — including
connection I/O errors, DNS resolution failures, pool acquisition timeouts, probe
timeouts, and any other SQLSTATE — MUST reset the consecutive-failure count to zero.

This requirement is the safety boundary that keeps credential recovery from firing on
database or network unavailability, for which restarting the hub is not a remedy.

#### Scenario: Transient DNS or network failure does not terminate the hub

- **WHEN** the database host cannot be resolved or reached
- **AND** the watchdog's probe fails repeatedly for that reason
- **THEN** the consecutive-failure count is reset on each such failure
- **AND** the hub continues running regardless of how long the condition persists

#### Scenario: Database unavailable does not terminate the hub

- **WHEN** Postgres is shutting down, starting up, or refusing connections
- **THEN** the probe failure does not count as an authentication failure
- **AND** the hub continues running

#### Scenario: Interleaved failures do not accumulate

- **WHEN** an authentication failure is followed by a non-authentication failure
- **AND** an authentication failure follows that
- **THEN** the consecutive count reflects only the most recent uninterrupted run of
  authentication failures
- **AND** the hub does not exit until that run reaches the strike limit

#### Scenario: A successful probe clears prior strikes

- **WHEN** the watchdog has recorded authentication failures below the strike limit
- **AND** a subsequent probe succeeds
- **THEN** the consecutive-failure count is reset to zero

### Requirement: Detection uses a connection established for the probe

The watchdog SHALL establish a new database connection for each probe rather than
acquiring one from the hub's shared pool, and SHALL bound each probe with a timeout so a
hung connection attempt cannot stall the watchdog loop.

Probing with a fresh connection is required so that a credential change is detected
while already-established pooled connections still authenticate, allowing the hub to
restart before it begins failing requests rather than after.

#### Scenario: Rotation detected while pooled connections still work

- **WHEN** the database password has been rotated
- **AND** the hub's pooled connections were established before the rotation and remain
  usable
- **THEN** the watchdog's fresh connection is rejected with an authentication failure
- **AND** detection does not wait for pooled connections to be recycled

#### Scenario: A hung connection attempt does not stall the watchdog

- **WHEN** a probe's connection attempt does not complete within the probe timeout
- **THEN** the probe is abandoned and treated as a non-authentication failure
- **AND** the watchdog continues probing on its interval

### Requirement: Recovery is delegated to the process supervisor

The hub's non-zero exit SHALL be sufficient for an external supervisor to restore
service by relaunching it, without any hub-side credential-refresh logic. The hub
therefore requires that its deployment resolve the database credential at process start.

#### Scenario: Supervised relaunch restores service on the new credential

- **WHEN** the hub has exited because of a sustained authentication failure
- **AND** its supervisor relaunches it via a launcher that re-resolves the credential
  from the secret store
- **THEN** the hub starts with the current password and serves normally

#### Scenario: Startup with an invalid credential also exits non-zero

- **WHEN** the hub starts and cannot connect to Postgres because the credential is
  invalid
- **THEN** startup fails and the process exits with a non-zero status
- **AND** no watchdog strike accumulation is required to reach that outcome

### Requirement: Health reporting is unchanged while the hub is alive

The credential watchdog SHALL NOT alter the behaviour of the hub's health endpoints. For
as long as the process is running, health reporting reflects database reachability
exactly as it did before, so existing external monitoring keeps working unchanged.

#### Scenario: Health endpoint still reports a degraded database

- **WHEN** the database is unreachable or rejecting authentication
- **AND** the hub process is still running
- **THEN** the liveness endpoint reports the database as down with an unavailable status,
  as it did prior to this change

