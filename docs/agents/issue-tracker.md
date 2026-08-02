---
type: reference
title: "Issue tracker: Gitea (ac/claude-code-history-viewer)"
description: "Where issues live for this fork, why they are NOT on either GitHub remote, and this repo's deltas from the house-wide gitea-backlog conventions."
tags: [agents, gitea, backlog, issues, fork]
timestamp: 2026-08-02
---
# Issue tracker: Gitea

Issues for this repo live in **Gitea Issues** at **`ac/claude-code-history-viewer`**
(the `internal` remote).

**Not GitHub — and this is the trap this file previously fell into.** The repo has
three remotes, two of them GitHub:

| remote | points at | role |
|---|---|---|
| `internal` | Gitea `ac/claude-code-history-viewer` | **the real backlog** |
| `origin` | GitHub `colangelo/claude-code-history-viewer` | our **public** fork — a publishing surface, not a tracker |
| `upstream` | GitHub `jhlee0409/claude-code-history-viewer` | **a third party's repo** |

Until 2026-08-02 this file declared the tracker to be `upstream`. Any skill reading
it (`/triage`, `/to-tickets`, `/to-spec`) would have filed *our* backlog items as
issues on **someone else's public repo**. The stock `gh` recipes made it worse: `gh`
resolves against whichever GitHub remote it picks, so the target was not even
deterministic between `origin` and `upstream`.

Upstream-facing work — changes we intend to send to `jhlee0409` — is tracked
**here**, under `area/upstream`. Their issue list is theirs, not our backlog; we
open a PR there when the work is ready.

> **`origin` is public.** Never write an internal hostname, a private IP, or a real
> home path into a tracked file in this repo — it lands in a public tree and stays
> in history. That is why this file names the Gitea *slug* but not the instance
> host: the host lives in the skill below, which is internal-only. House rule:
> CONTEXT `PATTERNS/paths-in-tracked-files.md`.

## The mechanics are house-wide — read them there, not here

Everything about *how* to drive this tracker is identical across the Gitea-backed
repos and is documented once, canonically, in the **`gitea-backlog` skill**:

- `~/_sync/dev/CONTEXT/SKILLS/gitea-backlog/SKILL.md` — commands, auth, the
  scoped-label model, derived views, the standing rules, the common mistakes
- `~/_sync/dev/CONTEXT/SKILLS/gitea-backlog/general-schema.toml` — the cross-project
  label vocabulary (`type/ status/ horizon/ needs/`, `reserved`)

(Reachable as `~/.config/claude/skills/gitea-backlog/`, a symlink into CONTEXT.)

Read that skill for the invocation, the flag-ordering trap (global flags go
**before** the subcommand), the `--repo` default trap (it defaults to a different
repo, so omitting it misfiles silently), `schema sync`, `doctor`, `roadmap`,
`kanban`. **Don't re-inline it here** — duplicated copies across repos drift, which
is the exact failure this repo already records for the Claude/Codex globals.

Bound to this repo:

```bash
SK=~/.config/claude/skills/gitea-backlog/gitea_backlog.py
GB="python3 $SK --repo ac/claude-code-history-viewer --project-dir ."
```

## This repo's deltas from the general schema

Declared in [`backlog-schema.toml`](../../backlog-schema.toml) at the root:

| | |
|---|---|
| `area/` values | `hub` · `daemon` · `providers` · `viewer` · `secondloop` · `upstream` · `ci` · `docs` · `meta` |
| `priority/` | **disabled** — single-user repo, `horizon/` suffices |

Everything else — `type/`, `status/`, `horizon/`, `needs/`, "never invent a label",
"done = a closed issue" — comes from the general schema unchanged.

Triage-role → label-string mapping: [`triage-labels.md`](./triage-labels.md).

## Not the issue tracker: the agent relay

Issues labelled **`agent-relay`** (plus `agent-working` / `agent-blocked` /
`relay-interactive`) are the cross-repo message channel, not backlog. They are
`reserved`, so they are excluded from every derived view. A backlog item is never
`agent-relay`. This repo is a **`nats`-variant participant** with no committed relay
surface — see `AGENTS.md` § Agent relay, and handle traffic with `/check-relay`.

## Branch policy for PRs

Two flows, and the previous version of this file stated only the second — as though
it governed everything:

- **Fork work (the default).** `main` is the integration *and* release line. Changes
  land on `main` directly, or via short-lived `feature/*` worktree branches that
  merge back into it. **There is no fork `develop` gate**, and a PR is not normally
  involved at all.
- **Contributing back to upstream.** Upstream uses `feature/* → develop → main`, so
  an upstream PR branches from `upstream/develop` and targets it.

So the old rule here — "the base branch MUST be `develop`, never `main`" — is true
*only* for upstream-facing PRs and wrong for everything else. `AGENTS.md` §
Branch Strategy is authoritative.

## Comment language

Applies to **upstream** PRs and to any issue opened on a public GitHub repo, where
the audience is other contributors:

- **Default is English**, regardless of the body's language, so the public review
  record stays consistent.
- **Exception for close-comments**: when closing an issue as a courtesy to its
  reporter, match the issue body's language.

Internal Gitea issues are single-user; write them in whatever is clearest.
