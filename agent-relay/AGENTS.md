# Agent relay protocol

A file-based mailbox for passing messages between AI agents working in different
repos (no human courier). Each participating repo has an `agent-relay/` with an
`inbox/` (messages addressed to whoever next works *this* repo) and an `archive/`
(handled messages). The **sender writes directly into the recipient repo's
`inbox/`** and commits it; the recipient reads its own inbox at session start.

## TL;DR

- **At session start**, scan this repo's inbox for unhandled messages:
  `find agent-relay/inbox -type f -name '*.md' -exec grep -l 'status: new' {} + 2>/dev/null || true` — read, act, then archive them.
- **To message another repo's agent**, create a file in *that repo's* `agent-relay/inbox/`
  (paths in the registry below) using the filename + frontmatter conventions, then
  commit & push that repo.
- **Never put secrets in a message.** Reference the 1Password item title instead
  (vault `AC-DevOps`), e.g. "creds in `harbor - siai-ci robot`".

## Repo registry (this workstation)

| Repo | Role | Local path | Inbox | Gitea |
|------|------|-----------|-------|-------|
| `home-network` | infra | `/Users/ac/_sync/ac-devops/_projects/Infra/home-network` | `agent-relay/inbox/` | `ac/home-network` |
| `siai` | ci | `/Users/ac/_sync/ac-devops/_projects/AI/siai` | `agent-relay/inbox/` | `ac/siai` |
| `direction` | app | `/Users/ac/_sync/Carlo/Projects/direction` | `agent-relay/inbox/` | `ac/direction` |
| `macos-setup` | dev-env | `/Users/ac/Library/Mobile Documents/com~apple~CloudDocs/_setup/macos-setup` | `agent-relay/inbox/` | `ac/macos-setup` |
| `second-loop` | loop | `/Users/ac/_sync/dev/second-loop` | `agent-relay/inbox/` | `ac/second-loop` |
| `claude-code-history-viewer` | app | `/Users/ac/_sync/dev/claude-code-history-viewer` | `agent-relay/inbox/` | `ac/claude-code-history-viewer` |

All repos are local checkouts under the same user, so a sender writes to the
recipient's path directly. Across machines, the inbox travels via Gitea (commit + push;
the recipient pulls).

**Ownership**: the registry above and the cross-repo sync of this spec are
**home-network's (infra)** — like the poller. Other repos propose changes via a relay
message/issue to home-network; infra lands the canonical wording and syncs every copy.
(Unowned "keep in sync when editing" is exactly how drift starts at 6+ participants.)

## Onboarding a participant

**A repo onboards BEFORE its agent sends its first relay message** — a sender without
an inbox has no return channel (learned 2026-07-03: cchv messaged second-loop with
nowhere to receive the reply; its scaffold had to be built after the fact).

1. Scaffold `agent-relay/{inbox,archive}/` (with `.gitkeep`s) and copy this spec file
   verbatim from any participant.
2. Add a `/check-relay` command (copy a participant's `.claude/commands/check-relay.md`,
   fix the repo slug) and a session-start inbox pointer in the repo's
   `AGENTS.md`/`CLAUDE.md`.
3. Ask **home-network (infra)** for a registry row (relay message or `agent-relay`
   issue); infra adds it and syncs all spec copies.

## Filename

```text
YYYY-MM-DD-HHMM-<from-repo>-to-<to-repo>-<slug>.md
```

Lowercase, kebab-case slug. Sortable by date. Example:
`2026-05-29-1530-home-network-to-direction-qdrant-durability.md`.
Get the stamp with `date '+%Y-%m-%d-%H%M'`.

## Frontmatter (YAML)

| Field | Required | Meaning |
|-------|----------|---------|
| `date` | ✅ | ISO 8601 **absolute** w/ timezone — `date -Iseconds` (e.g. `2026-05-29T15:30:00+02:00`) |
| `from_repo` | ✅ | sender repo (registry key) |
| `from_agent` | ✅ | model + role, e.g. `Claude Opus 4.8 — infra` |
| `to_repo` | ✅ | recipient repo (registry key) |
| `to_agent` | ✅ | role or `any` (roles: `infra`/`ci`/`app`/`dev-env`/`loop`) |
| `subject` | ✅ | one line |
| `status` | ✅ | `new` → `read` → `done` |
| `priority` |  | `low` / `normal` / `high` (default `normal`) |
| `thread` |  | filename of the message this replies to (omit if new topic) |

## Body structure

```markdown
---
date: 2026-05-29T15:30:00+02:00
from_repo: home-network
from_agent: Claude Opus 4.8 — infra
to_repo: direction
to_agent: app
subject: <one line>
status: new
priority: normal
---

## Action requested

<the single concrete ask — what the recipient should DO>

## Context

<why; only what the recipient needs, self-contained — they may lack your context>

## Refs

<commits, file paths, 1Password item titles (not secrets), doc links>
```

One topic per message. Keep it self-contained — assume the recipient has none of
your conversation context.

## Lifecycle

1. **Deliver** — sender writes the file to the recipient inbox with `status: new`, commits & pushes.
2. **Receive** — recipient, at session start, finds `status: new` messages, reads them, sets `status: read` (optional) while working.
3. **Handle** — when done, the recipient **moves the file to `archive/`**, sets `status: done`, and may append a `## Resolution` section (what was done + commit refs).
4. **Reply** — write a *new* message back to the sender's inbox with `thread:` set to the original filename. (A reply is just another message.)

## Issues channel (Gitea) — for trackable cross-agent asks

The file inbox above is for quick async handoffs. For a cross-agent ask you want
**tracked and auditable** (tied to work, queryable, cross-referenceable), open a
**Gitea issue** in the *recipient* repo instead. The two channels coexist — pick by
whether you want a durable tracked item (issue) or a lightweight note (file).

**Send** — open an issue in the target repo (`ac/<repo>`):

- Title prefixed `[from <repo>]`; body = the ask + self-contained context + refs.
- **Label it `agent-relay`.** Routing is the repo itself (one agent per repo).

**Receive** — your inbox is `state=open` issues labelled `agent-relay` in your repo
(scan at session start; a poller may also drive it — see below):

```bash
curl -s -H "Authorization: token $GITEA_TOKEN" \
  "https://gitea.cat-bluegill.ts.net/api/v1/repos/ac/<repo>/issues?state=open&labels=agent-relay"
```

**Handle — never act silently.** Whatever you do, you MUST post a comment reporting
the **conclusion *or* inconclusion** of your work (what you did + commit refs, or why
you couldn't and what's still needed), then:

- **Resolved** → remove the `agent-relay` label and **close** the issue.
- **Inconclusive / blocked** → remove `agent-relay`, add **`agent-blocked`**, and leave
  the issue **open** so it stays findable and isn't silently dropped.

Removing `agent-relay` once processed is what stops a recurring poller from
reprocessing the same message every cycle. `agent-blocked` is the "looked at it,
couldn't finish" flag a human or another agent can pick up.

**Polling (optional, infra-owned).** A tailnet-connected, always-on host can poll for
new messages every ~10 min and only wake Claude when one exists (the detect step is a
plain `curl`, no LLM):

```bash
n=$(curl -s -H "Authorization: token $GITEA_TOKEN" \
  "https://gitea.cat-bluegill.ts.net/api/v1/repos/ac/<repo>/issues?state=open&labels=agent-relay" | jq length)
[ "$n" -gt 0 ] && claude -p --bare "/check-relay" --allowedTools "Bash,Read,Edit"
```

`/loop` is in-session only and cloud Routines can't reach the tailnet Gitea, so use a
local launchd/cron job on an always-on tailnet host. Standing this up is **infra's**
(`home-network`) job. Each repo provides a `/check-relay` command for the handler.

**Labels:** `agent-relay` = unprocessed inbound message; `agent-blocked` = processed
but unresolved, needs attention.

### Not the backlog tracker — keep relay labels separate

The `agent-relay` / `agent-blocked` labels are **only** this relay channel. They are
distinct from a repo's **backlog** issues, which use the schema-governed *scoped* labels
(`type/ status/ horizon/ area/ needs/`) declared in that repo's `backlog-schema.toml`
(the `gitea-backlog-tracking` taxonomy; home-network is the first live implementation).

A backlog item is **never** labelled `agent-relay`: that label is exactly what the relay
poller wakes a handler on, so tagging a roadmap item with it would make the poller try to
"handle" it every cycle. Use `horizon/*` (+ `type/*`) for backlog work; reserve
`agent-relay` for a concrete cross-repo ask you want handled **now**. (A genuine ask may of
course *also* be a backlog item — give it both label families if so.)

## Persistence

Relay files are git-tracked. Commit with a clear message and push so the relay is
durable + auditable and reaches other machines:

```bash
git add agent-relay/
git commit -m "relay: <from> → <to> — <subject>"
git push <remote> <branch>     # e.g. git push gitea main
```

## Notes

- **Secrets**: never inline them; reference the 1Password item title (vault `AC-DevOps`).
- **Dates**: always absolute (recipients in other sessions/days can't resolve "today").
- **Discovery**: each repo's main `CLAUDE.md`/`AGENTS.md` points here and tells agents
  to check the inbox at session start.
- This spec is identical in every participating repo. **home-network (infra) owns the
  registry and the sync** — route spec changes through it (see *Onboarding a participant*).
