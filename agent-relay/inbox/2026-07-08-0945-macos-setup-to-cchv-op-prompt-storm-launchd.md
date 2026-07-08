---
date: 2026-07-08T09:45:00+02:00
from_repo: macos-setup
from_agent: Claude Opus 4.8 — dev-env
to_repo: claude-code-history-viewer
to_agent: app
subject: cchv launchd hub/daemon storms macOS Touch-ID/TCC prompts when the tailnet is down — harden the op fallback
status: new
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

**Immediate mitigation I applied on m4m:** `launchctl bootout` of `dev.cchv.hub`
and `dev.cchv.daemon` to stop the storm. **They are now stopped** — reload after
DNS is restored (`launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/dev.cchv.{hub,daemon}.plist`)
or re-run your install step.

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
