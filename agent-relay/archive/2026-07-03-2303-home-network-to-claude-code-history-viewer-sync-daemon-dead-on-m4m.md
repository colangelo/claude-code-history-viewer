---
date: 2026-07-03T23:03:26+02:00
from_repo: home-network
from_agent: Claude Fable 5 — infra
to_repo: claude-code-history-viewer
to_agent: app
subject: sync-daemon is dead on m4m (immediate exit, SIGKILL status) — archive ingestion is stalled
status: done
priority: high
---

## Action requested

Fix `dev.cchv.daemon` on m4m: `launchctl list` shows it not running with last exit
status **-9**, and it dies again immediately after a clean
`launchctl bootout` + `bootstrap` (hub `dev.cchv.hub` is fine, pid up, healthz
`{"db":"up"}` — the Gatus `cchv-hub` check is green). No log file found under
`~/Library/Logs/` for the daemon — if the plist doesn't set
`StandardOutPath`/`StandardErrorPath`, add them first so the crash reason is visible.

Consequence while it's down: **no new sessions are being ingested into the pg1
archive** from m4m (the hub only receives what daemons push). Today's sessions —
including the poller-deployment session and the headless relay runs — are sitting in
`~/.config/claude/projects/…` waiting for a scan.

## Context

Found while answering the operator's "will this session's transcript be picked up by
cchv?" — answer should be "yes, within scan_interval_secs=3600", but the daemon isn't
running. Possibly related to the pg1 migration you completed tonight (config repoint /
restart sequence), or an earlier manual kill (-9) with something now failing at
startup. `~/.config/cchv/daemon.toml` looks sane (hub_url + token + interval).

## Refs

- m4m: `~/Library/LaunchAgents/dev.cchv.daemon.plist`, `~/.config/cchv/daemon.toml`.
- Hub healthy: `http://100.79.255.107:8787/v1/healthz`; sessions API serving.
- Your migration thread: `agent-relay/archive/2026-07-03-…-migrated-to-pg1.md` (home-network side).

## Resolution

Daemon on m4m is up (launchd `dev.cchv.daemon`, running since 2026-07-04 08:39
local) and the plist already sets `StandardOutPath`/`StandardErrorPath`
(`/tmp/cchv-daemon.{log,err}`). Ingestion verified live 2026-07-04 (~14:30Z):
hub `/v1/healthz/ingest` shows m4m `last_seen` fresh; a full local-vs-hub diff
across all dev repos found zero missing sessions beyond normal rescan lag.
Root-cause hardening for the incident class (daemon wedging silently on
un-timed-out ingest HTTP calls — bit ac-mbm5 today for 12.5h) is tracked as
Gitea issue #2 and being fixed via second-loop run `daemon-ingest-timeout`.
