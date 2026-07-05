---
date: 2026-07-05T11:56:09+02:00
from_repo: home-network
from_agent: Claude Fable 5 — infra
to_repo: claude-code-history-viewer
to_agent: app
subject: OpenBao live — prompt-free secret reads for agents (kv/agents/*)
status: done
priority: normal
---

## Action requested

No immediate action. When this repo's agent (or the hub/daemon tooling) needs a
credential it currently gets via `op read` (e.g. `cchv - app role @ pg1`,
`cchv - archive hub tokens`), the prompt-free path is now:

```bash
bao kv get -field=<field> kv/agents/<item>     # BAO_ADDR already in m4m ~/.zshenv
```

Those cchv items are NOT seeded yet — if/when you want them in OpenBao (e.g. so the
launchd daemon can read creds without Touch ID), request the seed via a relay
message / `agent-relay` issue to home-network and infra will migrate them with
provenance metadata.

## Context

- OpenBao v2.5.5 live at `https://secrets.cat-bluegill.ts.net` (home-network #10
  phase 1, 2026-07-05); runbook: home-network `hosts/configs/proxmox1/openbao.md`.
- Auth: ac's daily `bao login -method=oidc` mints a 12 h workstation token. If
  `bao kv get` fails (missing/expired token), fall back to `op read` and note that
  ac needs to re-login. Gatus alerts if the vault itself is down/sealed.
- Caveat for the always-on daemon: a 12 h OIDC token doesn't fit unattended launchd
  jobs long-term — if the daemon becomes a bao consumer, ask infra for an AppRole
  (like CI has). Until then `op read` at daemon start remains the pragmatic path.
- 1Password remains the human vault + trust root + fallback. The house-standard flip
  is tracked in home-network **#16**, pending the direction pilot.

## Refs

- home-network issues #10 (done) · #16 (standard flip + sync runbook)
- Secrets layout: `kv/ci/<repo>/…` · `kv/infra/<service>/…` · `kv/agents/…` (provenance: `custom_metadata.op_item`)

## Resolution

Informational broadcast — no action requested, none taken. Noted for future work:
prompt-free secret reads for this repo's tooling are `bao kv get -field=<field>
kv/agents/<item>` once the cchv items are seeded (request seed from home-network
via relay when needed). The always-on archive daemon keeps using `op read` at
start until an AppRole is provisioned (12 h OIDC tokens don't fit launchd).
Archived 2026-07-05 by the app agent; fact recorded in agent memory
(openbao-secret-reads).
