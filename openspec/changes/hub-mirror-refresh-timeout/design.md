# hub-mirror-refresh-timeout — Design

## D1. Bound the attempt, not the query

The hang is a blocked socket read inside sqlx. Three layers could bound it.

**Chosen: a `tokio::time::timeout` around the whole refresh attempt.** It is the
only one of the three that covers *every* way an attempt can fail to return —
socket, DuckDB, or a future bug in the refresh path — and it needs no support
from Postgres or the OS. Cancellation is safe here for a specific reason: the
single-flight latch is an RAII `InFlight` guard, so dropping the future releases
it exactly as an early `?` does, and sqlx does not return an abandoned
connection to the pool. Refresh is already documented as "safe to interrupt" —
it is watermark-driven and idempotent, so a cancelled attempt costs re-read rows
on the next tick and nothing else.

**Rejected: a server-side `statement_timeout`.** It would not have fired here.
The backend had already been reaped — there was no server left to enforce it.
It bounds a genuinely slow query, which is a different fault, and it would have
to be set above the cold build's real duration to be safe.

**Rejected: TCP keepalive.** It attacks the true root cause (a half-open socket
is invisible without it) and would eventually surface an error. But sqlx 0.8's
`PgConnectOptions` exposes no keepalive knob, so this means reaching under the
pool; and even at a brisk interval it converts a permanent wedge into a
multi-minute one, which the ceiling in D2 already bounds more tightly. Worth
revisiting if half-open sockets show up elsewhere in the hub.

## D2. Two ceilings, because a cold build is not a slow refresh

A single ceiling cannot serve both cases. An incremental refresh is seconds; a
cold build is the whole archive and takes minutes. One value low enough to be a
useful bound on the first would cancel the second on every tick, and a cold
build that is cancelled on every tick never finishes — `/v1/stats/*` would
answer 503 permanently. That is a worse outage than the one being fixed, so the
two are configured separately and `run_refresher` picks by `mirror.is_empty()`.

Defaults are backstops, not budgets: 900 s incremental (two orders of magnitude
above a healthy refresh, and still inside the 3600 s staleness threshold so a
wedged tick is cancelled and retried *before* the mirror is old enough to page
anyone) and 21600 s cold build.

A partially-completed cold build that times out is picked up as an increment on
the next tick, since the watermark advanced — no special case needed.

## D3. `TimedOut` is an outcome, not an error

`refresh_bounded` returns `Ok(RefreshOutcome::TimedOut)` rather than `Err`.
Nothing failed and nothing is known — collapsing that into the error path would
log a cancelled attempt as a database fault and lose the distinction that
matters when reading the logs afterwards. It also keeps the existing guarantee
intact by construction: neither path can reach the credential watchdog, so a
timeout can no more restart the process than a failure can.

## D4. Testing a hang without Postgres

The regression test points the pool at a local listener that accepts and then
never speaks. TCP connects, so nothing fails, but no byte of the handshake
arrives — the production shape (`ESTABLISHED` with no live backend) reduced to
something deterministic. The distinction being tested is *silence*, not refusal;
a refused connection exercises the already-covered `Err` path. `connect_lazy`
plus an `acquire_timeout` far above the test's budget ensures it is the refresh
ceiling that fires and not sqlx's.

Two assertions, and the second is the one that bites: the attempt returns
`TimedOut`, **and a second attempt also returns `TimedOut` rather than
`Skipped`** — proving the latch was released. A fix that unblocked the loop but
leaked the latch would pass the first assertion, look healthy in the logs, and
leave the mirror just as frozen. The test needs no Postgres, so it runs in CI
unconditionally.
