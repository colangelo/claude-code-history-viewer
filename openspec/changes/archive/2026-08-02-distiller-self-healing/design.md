# distiller-self-healing — Design

## Context

The hub's journal layer is already self-healing at the data level: pending
groups are derived from the archive itself, dirty-detection is snapshot-exact
(`pg_visible_in_snapshot` against `journal_entries.generated_snapshot`), and
`POST /v1/journal/entries` is an idempotent upsert echoing the pending `as_of`.
A missed run is *just still pending on the next run*.

The scheduler layer, however, undoes this:

1. **DST day-close race.** `crates/hub/src/journal.rs` closes a logical day at
   `DAY_START_HOUR = 4` (04:00 UTC), and `GET /v1/journal/pending` excludes the
   still-open day. `dev.cchv.distiller.plist` fires at 05:30 *local* — 03:30
   UTC under CEST, 30 minutes before yesterday closes. Every DST run distills
   only days ≤ two days old; the feed is permanently ~2 days stale. (Under
   winter CET, 05:30 local = 04:30 UTC and the race disappears — which is how
   the bug hid at design time.)
2. **Single tick, no retry.** A transient failure (2026-07-23: bao DNS
   timeout, then a pg flake 500ing the pending query) aborts the run; launchd
   doesn't refire until tomorrow → +24h per incident.
3. **24h heal granularity.** Late data (sleeping laptop ingests at wake)
   correctly re-pends its day, but the heal waits for the next nightly tick.

Observed compound effect: newest entry 07-22 on 07-24, with 07-23 due 07-25.
There is also no monitoring: a stalled feed is only noticed by eye.

## Goals / Non-Goals

**Goals:**

- Entries for a closed logical day appear within ~1h of the 04:00 UTC close.
- A transient failure costs ≤1h, not 24h; no failure mode requires manual
  intervention to recover.
- Late-arriving (laptop-wake) data is re-distilled within ~1h of ingest.
- A stalled journal pipeline pages via Gatus instead of relying on eyeballs.

**Non-Goals:**

- Same-day ("live") distillation of the still-open logical day.
- Changing `DAY_START_HOUR`, the pending query, dirty-detection, entry schema,
  prompts, models, or backends.
- Event-driven push (NATS) triggering — polling a cheap idempotent endpoint
  hourly meets the freshness target with far fewer moving parts.
- De-duplicating re-distills of a day that dirties repeatedly (accepted cost,
  see Risks).

## Decisions

### D1: Hourly launchd `StartInterval` ticks, not a daemon, not events

Replace `StartCalendarInterval {05:30}` with `StartInterval 3600` (keep
`RunAtLoad true` — post-boot catch-up for free). An idle tick is one loopback
`GET /v1/journal/pending` and exit 0 — no LLM calls, no bao round-trip beyond
what the tick needs.

- *Why not fix the calendar time (e.g. 06:30 local)?* It heals failures next
  morning at best and late-laptop data still waits a day; it also keeps a
  hidden UTC/local coupling that DST already broke once.
- *Why not `--daemon` mode (KeepAlive loop)?* A wedged in-process loop is a
  new silent-stall mode — exactly the class being fixed. launchd already is
  the reliable scheduler.
- *Why not NATS events?* Freshness beyond requirement; needs subscription
  liveness monitoring plus a periodic sweep as backstop anyway.
- *Stampede history (#13):* moot — the default backend is aiproxy (HTTP), not
  `claude -p`; idle ticks make zero LLM calls; bao flakes ride the existing
  cached-token floor.

### D2: One `_with_retry` helper around hub HTTP calls

3 attempts, 30s sleep, on `requests.RequestException` and HTTP 5xx, applied to
`pending`, `session_messages` pagination, and `post_entry`. Everything else
(token resolution, LLM error handling, validation) is untouched — those paths
already have their own floors/retries. Exit 1 on final failure remains, but now
means "next tick retries in ≤1h".

### D3: `GET /v1/healthz/journal` judges *data sitting undrained*, not runner liveness

Unauthenticated, mirroring `/v1/healthz/ingest`'s shape (Gatus consumes status
code + body). Semantics:

- Compute pending groups for **closed** logical days (same CTE semantics as
  `journal::pending`, same `DAY_START_HOUR` fold), each joined with its latest
  data arrival `max(messages.created_at)` over the group's sessions.
- **Bound to the distiller's forward horizon** (`within_days`, default 7,
  matching the distiller's `--horizon-days`): only groups with
  `entry_date >= logical_today - within_days` are evaluated. This is essential,
  not cosmetic — the archive holds **hundreds of pending closed-day groups
  going back months** (197 older than 7 days as of 2026-07-24, back to
  2026-04-05) that the distiller *never auto-distills*; they await an explicit
  `--backfill` and are not a stall. Without this bound the endpoint would be
  503 forever and monitor nothing.
- A group is `stale` when `now - latest_arrival > grace_secs` (default 7200,
  `?grace_secs=`/`?within_days=` overrides parsed string-first → 400 on
  garbage, same pattern as `stale_after_secs`). Any in-window stale group →
  503 `"stale"`, else 200 `"ok"`; body lists the evaluated (in-window pending)
  groups for observability.
- *Why two params?* `within_days` = "which days are the distiller's
  responsibility to drain soon"; `grace_secs` = "how long after data arrives
  before undrained counts as a stall". Orthogonal: a day inside the horizon
  that got data 5 min ago is pending-but-fresh (green); the same day pending
  3 h is stale (red).
- *Why arrival-based grace instead of "pending exists"?* A day re-dirtied by a
  20:00 laptop wake is legitimately pending for up to an hour; grace keeps
  that green while the hourly tick drains it. *Why not a distiller dead-man
  ping (Healthchecks.io)?* It only detects the runner dying — today's bug had
  succeeding runs that distilled nothing; arrival-based staleness catches all
  stall modes including scheduling/query bugs.
- Named `journal-health` capability; the endpoint lives in
  `crates/hub/src/health.rs` next to the ingest check, with the group-CTE
  shared knowledge referenced from `journal.rs` (comment cross-links, not
  premature extraction — the two queries differ in join target).

### D4: Versioning + deploy path

New endpoint ⇒ minor bump `cchv-v0.13.0` (package.json → `just sync-version`).
Hub swap on m4m follows `docs/archive/deployment.md` §2b (rm-first + ad-hoc
codesign + bootout/bootstrap — never `kickstart -k`). Plist + script reinstall
is local (`~/.local/bin/cchv-distill`, `~/Library/LaunchAgents`). Gatus check
is relayed to infra (home-network repo owns it) — same Host-header pattern as
the existing cchv-hub/cchv-ingest checks.

## Risks / Trade-offs

- [Re-distill churn: a day dirtying N times costs N cheap LLM calls] → bounded
  by hourly daemon ingest scans (realistically 1–3/day/group; gpt-5.6-sol at
  `effort=low`). If cost ever matters, add a quiescence window to the pending
  query later — hub-side, no distiller change.
- [Hourly bao AppRole logins (24×/day)] → negligible load; on flake the
  cached-token floor already carries the tick.
- [503 flaps if grace < drain latency during a big backfill] → grace default
  (2h) is 2× the tick interval, matching the ingest check's 2× heuristic;
  Gatus alerting thresholds (consecutive failures) damp the rest.
- [Healthz query cost: group CTE over all sessions per probe] → same order as
  the pending query the distiller already issues hourly; Gatus probes at
  minutes-scale intervals against a loopback-fast pg pool. Revisit with an
  index only if pg1 shows it in slow logs.
- [launchd `StartInterval` drifts across sleep (ticks skipped while asleep)] →
  `RunAtLoad` covers boot; m4m is always-on; the worst case on a sleeping
  machine is the next wake's tick — still ≤1h after wake.

## Migration Plan

1. Land hub endpoint + tests; release `cchv-v0.13.0`; stage + relay binary
   swap to infra (§2b).
2. Install updated script + plist locally on m4m (bootout/bootstrap); verify
   an idle tick logs "nothing pending" and a natural tick after 04:00 UTC
   drains yesterday.
3. Relay Gatus check addition to infra after the endpoint is live.
4. Rollback: previous hub binary stays in `~/.config/cchv/staging/`; plist
   rollback is re-adding the calendar trigger. No data migration in either
   direction — hub state is untouched.

## Open Questions

- None blocking. (Distiller runs on m4m only today; if a second distiller host
  ever appears, the idempotent upsert + `as_of` echo already tolerates racing
  drains, but the plist/install story would need its own pass.)
