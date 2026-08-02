# distiller-self-healing

## Why

The journal feed stalled for 2+ days (newest entry 07-22 on 07-24) because the
distiller's scheduling layer defeats the hub's self-healing design in three
compounding ways: (1) the nightly 05:30 *local* tick fires at 03:30 UTC during
DST — 30 minutes **before** the 04:00 UTC logical-day close — so every run can
only see days ≤ two days old; (2) a single daily tick with no retry turns any
transient failure (bao DNS flake, pg connection 500) into a +24h loss; (3)
late-arriving data (sleeping laptop) is correctly re-pended by dirty-detection
but only healed at the next daily tick. The hub layer (data-derived pending
list, snapshot dirty-detection, idempotent upsert) already supports arbitrary
re-drains — only the drain loop and its observability are missing.

## What Changes

- Replace the distiller's `StartCalendarInterval 05:30` with hourly
  `StartInterval` ticks; an idle tick is one loopback HTTP call and zero LLM
  calls. This removes the DST day-close race structurally and bounds staleness
  (day close, failures, late data) to ~1h.
- Add retry/backoff (3 attempts, 30s) to the distiller's hub calls
  (`pending`, `session_messages`, `post_entry`) so transient hub/pg flakes
  don't waste a tick.
- Add unauthenticated `GET /v1/healthz/journal` to the hub: 503 when any
  closed-logical-day pending group's latest data arrival is older than
  `grace_secs` (default 7200) — i.e. work has been sitting undrained. Mirrors
  `/v1/healthz/ingest`'s Gatus-consumable shape.
- Relay a one-line Gatus check for the new endpoint to infra (home-network).

## Capabilities

### New Capabilities

- `journal-health`: hub health endpoint reporting journal distillation
  staleness for external monitors (Gatus), with a grace window that tolerates
  in-flight hourly drains.

### Modified Capabilities

- `journal-entries`: the "Distiller job" requirement gains self-healing
  scheduling semantics — bounded staleness after logical-day close, retry on
  transient hub errors, idempotent frequent ticks replacing the single nightly
  run.

## Impact

- `scripts/dev.cchv.distiller.plist` (+ installed copy in
  `~/Library/LaunchAgents`): schedule change, reload via bootout/bootstrap.
- `scripts/cchv-distill.py` (+ installed `~/.local/bin/cchv-distill`): small
  retry helper; no token/backend/prompt changes.
- `crates/hub/src/health.rs` + route in `crates/hub/src/lib.rs`: new endpoint;
  hub minor version bump (`cchv-v0.13.0`) and m4m binary swap via infra relay
  per `docs/archive/deployment.md` §2b.
- Monitoring: infra adds the Gatus check (relay; no repo change here).
- No frontend, no Tauri, no daemon, no DB schema changes.
