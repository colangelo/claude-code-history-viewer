# distiller-tick-observability

## Why

`GET /v1/healthz/journal` returns 503 when closed logical days sit undrained. It
cannot say **why**, and the two causes need opposite responses:

- the distiller is running and working through a backlog — wait;
- the distiller has not run at all — fix the host or the job.

A 503 reads identically either way. Measured by infra on the hub machine,
2026-08-21 (relay thread `576c10a3`): the endpoint was 503 with 26 stale groups
at 04:41Z and **byte-identical** to a reading 33 minutes earlier, because no tick
had run since 00:56Z — `launchctl print` showed `state = not running`, 3 h 45 m
with zero ticks. Nothing in the response distinguished that from a slow drain,
and two separate derivations — one on each side of the relay — used the hourly
`StartInterval` to predict a clear-by time and missed it.

Two facts came out of that measurement, and both are load-bearing:

**1. `StartInterval 3600` is not hourly on a machine that sleeps.** launchd does
not fire intervals while asleep, a DarkWake is not a wake, and the whole run of
missed intervals is coalesced into **one** catch-up at the next full wake — not
replayed. The hub machine idle-slept at 01:29Z and cycled Sleep↔DarkWake through
02:26Z; the ~01:51Z tick never fired. That host sleeps 40–106×/day. So "N groups
÷ 50 per hourly tick" is a **floor, not an estimate**: the drain rate is set by
the wake schedule, not by the interval.

Our own docs assert the opposite in four places — `dev.cchv.distiller.plist`
("late laptop-wake data is drained within the hour"), `docs/archive/deployment.md`
§3c ("bounds journal staleness to ~1h"), `cchv-distill.py`'s retry comment ("a
tick that gives up is retried within the hour"), and the `journal-entries` spec
("some tick within the hour after close MUST pick it up"). The archived
`distiller-self-healing` design even lists the risk and dismisses it: *"m4m is
always-on; the worst case on a sleeping machine is the next wake's tick — still
≤1h after wake."* The first clause is false and the second measures from the
wrong event.

**2. A day-close is never a net drain of this check.** `grace_secs` counts from
each group's `latest_arrival`, so a group whose data landed more than
`grace_secs` before its day closed is stale **the instant the day closes**. The
04:00Z roll that retired 6 stranded groups admitted a newly closed day carrying
20, twelve already stale on arrival. Only a tick clears this check; the clock
never does. Any prediction of the form "it will go green at <wall-clock time>"
is unsound, and this is the second one this thread produced.

## What Changes

- **The distiller records each tick with the hub.** New `POST /v1/journal/ticks`
  (machine-token, same auth as the entry upsert), called once per non-dry-run
  invocation immediately after the pending query succeeds, carrying the mode
  (`forward` / `backfill`) and how many groups it found.
- **`GET /v1/healthz/journal` reports tick liveness** next to the stale-group
  count: `last_tick_at`, `last_tick_age_secs`, `last_tick_mode`,
  `last_tick_groups_pending`, and `ticks_last_24h`.
- **Alerting on tick age is opt-in.** A new `max_tick_age_secs` query param is
  **absent by default**, and then tick age is reported but never alerts —
  the `max_lag_rows` precedent from `/v1/healthz/stats`. On a host that sleeps
  40–106×/day, any default here is a guess about the wake schedule dressed up as
  a health rule. When it is set and exceeded (or no tick has ever been recorded),
  the endpoint returns 503 with status `"no_tick"`.
- **The cadence claims are corrected** in the plist, `deployment.md` §3c, the
  distiller's retry comment, and the `journal-entries` spec: the interval bounds
  staleness in **wakes**, not hours.
- **The day-close property is pinned** by a scenario and a test, so the next
  reader does not re-derive a wall-clock deadline for a check that has none.

Default behaviour of the endpoint is unchanged: with `max_tick_age_secs` absent,
the status code and verdict are exactly what they are today. Existing monitors
keep working untouched.

## Impact

- Specs: `journal-health` (new reported fields, opt-in alert, day-close
  scenario), `journal-entries` (distiller records ticks; corrected cadence
  guarantee).
- Code: `migrations/0007_distiller_ticks.sql`, `crates/hub/src/journal.rs`,
  `crates/hub/src/health.rs`, `crates/hub/src/lib.rs` (route),
  `scripts/cchv-distill.py`.
- Docs: `docs/archive/deployment.md` §3c, `scripts/dev.cchv.distiller.plist`.
- Rollout: hub swap **and** distiller reinstall on the hub machine. Until the
  distiller half lands the new fields report "never ticked", which is honest and
  alerts nobody (the param is opt-in). Relay to infra; a `max_tick_age_secs`
  value for the Gatus check is theirs to choose, and they own the related
  monitoring decision on `ac/infra#94`.
