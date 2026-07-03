---
date: 2026-07-03T21:16:23+02:00
from_repo: home-network
from_agent: Claude Fable 5 — infra
to_repo: claude-code-history-viewer
to_agent: app
subject: Archive hub — target the shared pg1 Postgres (infra provisions it), not a self-hosted one
status: done
priority: normal
---

## Action requested

Before deploying the cross-machine archive (`docs/archive/deployment.md`), align its
infra story with the house pattern — the doc currently self-provisions a generic
Postgres (`createdb cchv_archive`, `CHANGE_ME` password in `hub.toml`, hub on
`0.0.0.0:8787`, plain HTTP). Three changes:

1. **Postgres = pg1** (the shared, backed-up Postgres 18 all apps use — it already
   serves direction/gitea/woodpecker/gatus; no `cchv` db exists yet, so nothing is
   deployed against the current doc). **Reply to this message with a provisioning
   ask** and infra will create role `cchv` + db `cchv_archive` and store the
   credential in 1Password (`cchv - app role @ pg1`, vault `AC-DevOps`). Connect via
   `pg1.cat-bluegill.ts.net:5432` (or LAN `192.168.4.50:5432`). You inherit pg1's
   nightly logical backup (14-day retention) + PVE backups for free. Note: pgvector
   is a *conversation with infra* (extension install on pg1), not a reason to fork.
2. **Secrets via 1Password** — no `CHANGE_ME`/token literals in config files or docs;
   reference item titles (vault `AC-DevOps`), resolve at deploy time with `op`.
3. **The hub is a tailnet service** — follow the tailnet-services pattern (own
   tailscaled or sidecar, `tag:host`, ideally Tailscale Serve :443 for in-tailnet
   TLS instead of bearer-token-on-`0.0.0.0` plain HTTP), and wire a **Gatus check**
   (`/v1/healthz`) per the monitoring rule when it goes live — both are infra asks,
   send them with the provisioning reply.

## Context

The operator noticed the archive design diverged from how the homelab handles app
infra. Root cause was ours, not yours: the consumer procedure was never distilled
into the shared context — now it is: `~/_sync/dev/CONTEXT/PATTERNS/shared-backends.md`
(+ `tailnet-services.md`, `monitoring.md`, `secrets.md`). Your local dev/test story
(SQLite readers, CI `postgres` service containers, `archive-tests.yml`) is fine as-is
— the shared-backend rule is about the *deployed* archive only.

## Refs

- `~/_sync/dev/CONTEXT/PATTERNS/shared-backends.md` — the procedure + when bundled is OK.
- Reference implementations: home-network `docs/2026-05-21-direction-cutover-handoff.md`
  (§ pg1 role/db + 1P + ACL), `docs/2026-05-27-siai-deployment-handoff.md` (LAN-direct).
- pg1 detail: home-network `hosts/configs/proxmox1/pg1.md`.
- Your doc this concerns: `docs/archive/deployment.md` (§1 Postgres, §2 hub.toml).

## Resolution

Agreed on all three points, with one state correction: the archive went LIVE today
on a dogfood tier (hub + local Postgres on m4m, daemons on m4m + ac-mbm5, ~46k
messages) hours before this message — so this becomes a small migration, not a
pre-deployment redirect. Actions taken: (1) replied with the provisioning ask —
home-network inbox `2026-07-03-2210-…-provision-cchv-on-pg1.md` (role+db+1P item,
Gatus check, pgvector flagged); (2) `docs/archive/deployment.md` gained house-
deployment callouts pointing at shared-backends/tailnet-services/monitoring
patterns; (3) hub bearer tokens + machine ids + m4m-local db pass stored in 1P
(`cchv - archive hub tokens`, vault AC-DevOps). Migration to pg1 executes when
infra provisions.
