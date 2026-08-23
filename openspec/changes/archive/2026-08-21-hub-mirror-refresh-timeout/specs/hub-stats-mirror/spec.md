# hub-stats-mirror — Delta

## MODIFIED Requirements

### Requirement: Refresh failures degrade rather than escalate

A refresh that fails — including because Postgres is unreachable — SHALL leave
the existing mirror intact and the hub SHALL continue serving statistics from it.

The refresh path MUST NOT terminate the process and MUST NOT contribute to the
sustained-authentication-failure count that governs process exit, so that a
refresh failure is never mistaken for a rotated credential.

Each refresh attempt SHALL be bounded by a configurable wall-clock ceiling and
SHALL be cancelled when it exceeds it, so that an attempt which never returns is
degraded on the same terms as one that fails. A refresh does not always fail
loudly: a database connection that stops delivering without closing leaves the
attempt blocked with no error to report, and because the refresher sleeps only
after an attempt returns, one such attempt would otherwise stop all further
refreshes for the life of the process while statistics continued to be served
from a mirror frozen at the moment it hung.

Cancelling an attempt SHALL release the single-flight latch, so the following
scheduled refresh runs rather than yielding to the abandoned one. A cancellation
that left the latch held would keep the refresher just as wedged while reporting
each subsequent tick as a routine skip.

The ceiling SHALL be configurable separately for an incremental refresh and for
a cold build, because a cold build reads the entire archive and takes minutes; a
bound tight enough to be useful for an increment would cancel every cold build
before it completed, and a cold build that never completes leaves `/v1/stats/*`
answering `503` permanently.

#### Scenario: Statistics survive a database outage

- **WHEN** Postgres is unreachable and a scheduled refresh fails
- **THEN** the hub continues to answer `/v1/stats/*` from the existing mirror

#### Scenario: A failing refresh does not restart the hub

- **WHEN** refreshes fail repeatedly
- **THEN** the process does not exit as a result

#### Scenario: A refresh that stops responding is cancelled

- **WHEN** a refresh exceeds its configured wall-clock ceiling
- **THEN** the attempt is cancelled, the existing mirror is left intact, and the
  outcome is reported as a timeout rather than as a refresh failure

#### Scenario: A cancelled refresh does not block the next one

- **WHEN** an attempt has been cancelled for exceeding its ceiling
- **THEN** the next scheduled refresh acquires the mirror and runs, rather than
  being skipped as though a refresh were still in flight

#### Scenario: A cold build is not cancelled by the incremental ceiling

- **WHEN** the mirror is empty and a cold build is running
- **THEN** the attempt is bounded by the cold-build ceiling rather than the
  incremental one, so it is allowed to run to completion
