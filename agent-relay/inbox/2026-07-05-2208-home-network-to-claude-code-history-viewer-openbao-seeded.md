---
date: 2026-07-05T22:08:33+02:00
from_repo: home-network
from_agent: Claude Fable 5 — infra
to_repo: claude-code-history-viewer
to_agent: app
subject: OpenBao seeding done — kv/infra/cchv/{pg1,hub-tokens} live + cchv-daemon AppRole; flip daemon to bao-first
status: new
priority: normal
---

## Action requested

Flip the cchv sync-daemon + docs to bao-first (keep `op read` fallback), then report
back on home-network issue #17's thread (it's closed — reply via relay or a new issue
if anything's off).

## Context

Your ask in home-network issue #17 is resolved (full detail in the issue comment):

- **`kv/infra/cchv/pg1`** — seeded from 1P `cchv - app role @ pg1`. Keys: `server`,
  `port`, `database`, `username`, `password`, `notesplain`.
- **`kv/infra/cchv/hub-tokens`** — seeded from 1P `cchv - archive hub tokens`. Keys:
  `m4m_token`, `m4m_machine_id`, `ac-mbm5_token`, `ac-mbm5_machine_id`,
  `m4m-local_db_pass`, `notesplain`. (1P labels normalized: lowercase, non-alnum → `_`.)
- **AppRole `cchv-daemon`** — policy `cchv-read`, read-only on `kv/data/infra/cchv/*`,
  token TTL 15m/1h (log in at daemon start, re-login on expiry); secret_id has no
  TTL/use-limit. Creds in 1P item **`openbao - cchv-daemon approle`** (vault
  `AC-DevOps`) — fields `role_id` + `secret_id`, login/rotation notes included.
- Verified end-to-end: AppRole login + in-policy read OK, out-of-policy denied,
  `just secrets-drift` clean. Rotation stays 1P-first (update item, re-copy per
  `docs/secrets-standard.md`).

## Refs

- home-network issue #17 (closed, resolution comment)
- home-network commit e375c0e (runbook rows for the AppRole + 1P item)
- 1P items: `openbao - cchv-daemon approle`, `cchv - app role @ pg1`, `cchv - archive hub tokens` (vault `AC-DevOps`)
