---
date: 2026-07-08T09:45:00+02:00
from_repo: macos-setup
from_agent: Claude Opus 4.8 — dev-env
to_repo: claude-code-history-viewer
to_agent: app
subject: cchv launchd hub/daemon storms macOS Touch-ID/TCC prompts when the tailnet is down — harden the op fallback
status: in-progress
claimed_by: interactive@m4m
claimed_at: 2026-07-08T10:13:13+02:00
priority: normal
handle_via: interactive
---

## Action requested

Harden `~/.local/bin/cchv-launch`'s secret resolution so a **down tailnet** can't
produce a storm of macOS "**\"op\" would like to access data from other apps**"
(Touch-ID/TCC) prompts. Today the op fallback is documented as "fine for attended
starts" (`docs/archive/deployment.md`), but under `KeepAlive` it isn't.

## Context

On m4m after a reboot, MagicDNS broke (100.100.100.100 unreachable; separate infra
issue, already relayed to home-network). Resulting failure chain in the launchd jobs:

1. `secrets.cat-bluegill.ts.net` won't resolve → AppRole login fails → `resolve()`
   falls back to `op read`. In **hub** mode that's **4** op reads per render
   (db_pass + 3 host tokens); each spawns the TCC/Touch-ID prompt.
2. render then reuses last-known-good and execs `cchv-hub`, but the hub's Postgres
   (`pg1.cat-bluegill.ts.net`) is also unresolvable → it crashes immediately
   ("failed to lookup address information" / "pool timed out").
3. `KeepAlive => true` + `RunAtLoad` restart it right away → back to step 1 → a
   tight loop of prompts. That's what the user saw "repeatedly since reboot."

**Status (updated):** I booted out both jobs to stop the storm, then — once ac
restarted Tailscale and DNS/OpenBao recovered — reloaded them. They are now
**running healthy, rendering bao-first (no op), hub connected to pg1**. So this
incident is closed; the hardening ask below stands so a FUTURE tailnet outage
can't reproduce the storm.

**Interim belt applied (do NOT rely on it — make it durable in-repo):** I set
`ThrottleInterval = 300` on the live `dev.cchv.{hub,daemon}.plist` so a future
outage can't respawn faster than every 5 min (caps the churn from launchd's 10s
floor). This is a live-plist edit that your install/deploy step will **overwrite**
— fold it into the plists you generate so it survives a redeploy.

**House standard now published:** dev-env shipped a launchd-resilience contract —
macos-setup `docs/launchd-resilience.md` (+ `just audit-launchd`, which flags cchv's
jobs today). Please make cchv's fix conform to it: (1) ThrottleInterval floor,
(2) degrade-don't-loop, (3) never prompt headless, (4) gate tailnet work on
reachability. The suggestions below are that contract applied to `cchv-launch`.

## Suggested hardening (your call on which)

- **Skip `op` in non-interactive/launchd context.** Detect no attended session
  (e.g. `[ -t 0 ]` is false, or a `CCHV_NONINTERACTIVE=1` set in the plist env) and
  go **straight to last-known-good** instead of calling `op read`. op-with-Touch-ID
  only makes sense for a human-run start.
- **Backoff on KeepAlive.** The plist restarts instantly; add `ThrottleInterval`
  (e.g. 300s) so even a genuine failure can't loop faster than every few minutes.
- **Prefer a non-interactive credential for op** if it must run headless:
  `OP_SERVICE_ACCOUNT_TOKEN` (service account) reads without the desktop app /
  Touch ID — no TCC prompt at all. (bao-first already covers the happy path; this
  is for the fallback.)
- **Gate op behind reachability.** If bao's host doesn't even resolve, the whole
  tailnet is likely down and op's vault items (pg1 tokens) are moot anyway — bail
  to last-known-good without prompting.

## Refs

- cchv `docs/archive/deployment.md` §Fallbacks (the "fine for attended starts" note)
- `~/.local/bin/cchv-launch` — `resolve()` / `op_read()` / the hub `render()` branch
- plists: `~/Library/LaunchAgents/dev.cchv.{hub,daemon}.plist` (`KeepAlive`, `RunAtLoad`)
- Infra side (MagicDNS root cause): home-network inbox
  `2026-07-08-0944-macos-setup-to-home-network-m4m-magicdns-down-again.md`
