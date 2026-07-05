# AGENTS.md

Agent notes for this repo. Project/code guidance lives in `CLAUDE.md`; this file
carries house conventions that apply to agents working here.

## Secrets

Machine reads default to **OpenBao** — `bao kv get kv/<path>` (`BAO_ADDR` is
machine-wide; auth is ac's daily `bao login -method=oidc`, 12 h token). If the
token is missing/expired, fall back to `op read` and tell ac to re-login.
1Password stays the human vault and the fallback (vault `AC-DevOps`). Never
commit/inline a secret — reference a 1P item title or a bao kv path. Need a
seed / AppRole / ACL grant? Relay message or `agent-relay` issue to
home-network. Canonical: home-network `docs/secrets-standard.md`; portable
copy: CONTEXT `PATTERNS/secrets.md`.

Repo-specific: the cchv items (`cchv - app role @ pg1`, `cchv - archive hub
tokens`) are not seeded in OpenBao yet — the always-on archive daemon keeps
using `op read` at start until an AppRole is provisioned (12 h OIDC tokens
don't fit launchd).

## History archive ops

Archive deployment: `docs/archive/deployment.md`. Recovering history older
than Claude Code's ~30-day local retention from Time Machine backups (any
machine, incl. retired ones via their TM disk): `docs/archive/timemachine-backfill.md`
(`just tm-backfill --list` to see what's recoverable).

## Agent relay

Cross-repo messages arrive in `agent-relay/inbox/` and as Gitea issues labelled
`agent-relay`. Protocol: `agent-relay/AGENTS.md`. Relay content is committed to
the **`internal`** Gitea remote only — never pushed to the public GitHub remotes.
