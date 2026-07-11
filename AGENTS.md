# AGENTS.md

Agent notes for this repo. Project/code guidance lives in `CLAUDE.md`; this file
carries house conventions that apply to agents working here.

## House conventions & skills

Cross-project conventions and skills live in **`~/_sync/dev/CONTEXT/`** (progressive
disclosure — read an index, drill only what you need): `PATTERNS/index.md` for "how we
do X here" (Justfiles, git remotes, containers, secrets, docs, backlog, CI, second-loop,
…), `SKILLS/README.md` for actionable skills (read the matching `SKILL.md` and follow
it). Check PATTERNS before inventing a convention; check SKILLS before improvising a
procedure.

## Secrets

Machine reads default to **OpenBao** — `bao kv get kv/<path>` (`BAO_ADDR` is
machine-wide; auth is ac's daily `bao login -method=oidc`, 12 h token). If the
token is missing/expired, fall back to `op read` and tell ac to re-login.
1Password stays the human vault and the fallback (vault `AC-DevOps`). Never
commit/inline a secret — reference a 1P item title or a bao kv path. Need a
seed / AppRole / ACL grant? Relay message or `agent-relay` issue to
home-network. Canonical: home-network `docs/secrets-standard.md`; portable
copy: CONTEXT `PATTERNS/secrets.md`.

Repo-specific: the cchv secrets ARE seeded in OpenBao (home-network#17, done
2026-07-05) — `kv/infra/cchv/pg1` (hub DB creds) and `kv/infra/cchv/hub-tokens`
(per-machine hub tokens). The always-on archive jobs (daemon + hub) are
**bao-first** via `scripts/cchv-launch.sh` and the AppRole `cchv-daemon`
(creds file `~/.config/cchv/bao-approle`, from 1P item
`openbao - cchv-daemon approle`); `op read` is the fallback, a last-known-good
rendered config the floor. See `docs/archive/deployment.md` §3b. Flipped on
m4m 2026-07-05; ac-mbm5's daemon still runs the old static config — flip it
with the §3b per-machine steps next time an attended session is on ac-mbm5.

## History archive ops

Archive deployment: `docs/archive/deployment.md`. Recovering history older
than Claude Code's ~30-day local retention from Time Machine backups (any
machine, incl. retired ones via their TM disk): `docs/archive/timemachine-backfill.md`
(`just tm-backfill --list` to see what's recoverable).

## Agent relay

Cross-repo messages arrive in `agent-relay/inbox/` and as Gitea issues labelled
`agent-relay`. Protocol: `agent-relay/AGENTS.md`. Relay content is committed to
the **`internal`** Gitea remote only — never pushed to the public GitHub remotes.
