# Agent relay protocol

A file-based mailbox for passing messages between AI agents working in different
repos (no human courier). Each participating repo has an `agent-relay/` with an
`inbox/` (messages addressed to whoever next works *this* repo) and an `archive/`
(handled messages). The **sender writes directly into the recipient repo's
`inbox/`** and commits it; the recipient reads its own inbox at session start.

## TL;DR

- **At session start**, scan this repo's inbox for unhandled messages:
  `find agent-relay/inbox -type f -name '*.md' -exec grep -l 'status: new' {} + 2>/dev/null || true` — read, **claim** (set `status: in-progress`, commit & push), act, then archive.
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
| `macos-setup` | dev-env | `/Users/ac/_sync/dev/macos-setup` | `agent-relay/inbox/` | `ac/macos-setup` |
| `second-loop` | loop | `/Users/ac/_sync/dev/second-loop` | `agent-relay/inbox/` | `ac/second-loop` |
| `claude-code-history-viewer` | app | `/Users/ac/_sync/dev/claude-code-history-viewer` | `agent-relay/inbox/` | `ac/claude-code-history-viewer` |
| `sergente` | agent | `/Users/ac/_sync/dev/sergente` | `agent-relay/inbox/` | `ac/sergente` |

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
| `to_agent` | ✅ | role or `any` (roles: `infra`/`ci`/`app`/`dev-env`/`loop`/`agent`) |
| `subject` | ✅ | one line |
| `status` | ✅ | `new` → `in-progress` → `done` |
| `claimed_by` | ⏳ | required once `in-progress`: who is handling it, `<role>@<host>` (e.g. `infra-poller@m4m`, `interactive@ac-mbm5`) |
| `claimed_at` | ⏳ | required once `in-progress`: claim time, ISO 8601 absolute (`date -Iseconds`) |
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
2. **Claim — MANDATORY, before doing any work.** Set `status: in-progress`, add
   `claimed_by: <role>@<host>` and `claimed_at: $(date -Iseconds)`, then **commit & push
   immediately** (before starting the actual work). This is the lock: all scans filter on
   `status: new`, so a claimed message is invisible to other handlers — instantly for
   sessions on the same machine (poller vs interactive share the checkout), after the
   next pull elsewhere. **Never start work on a message someone else has claimed** (see
   *Stale claims* below for the one exception).
3. **Handle** — when done, the **same party that claimed it** moves the file to
   `archive/`, sets `status: done`, and appends a `## Resolution` section (what was done
   + commit refs). Never leave handled work unarchived — a dangling `new` re-triggers the
   poller every tick; a dangling `in-progress` goes stale and gets re-handled.
4. **Reply** — write a *new* message back to the sender's inbox with `thread:` set to the original filename. (A reply is just another message.)

**Stale claims.** A claim is a lease, not ownership: if `claimed_at` is older than
**2 hours**, any handler may take over — update `claimed_by`/`claimed_at` (commit & push,
same as a fresh claim) and note the takeover in the eventual `## Resolution`. This keeps a
crashed session from deadlocking a message.

**Cross-machine caveat.** The file-channel lock propagates via git, so two handlers on
*different machines* still race for up to one pull cycle (~one poller tick). Same-machine
races — the common case (poller + interactive session on one workstation) — close in
seconds. For asks where duplicate handling is expensive, prefer the issues channel below:
its label-swap claim is near-atomic (Gitea is a single source of truth, no sync lag).

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

**Claim — MANDATORY, before doing any work.** Swap the label `agent-relay` →
**`agent-working`** (and optionally comment `claimed by <role>@<host>`). Both the poller
gate and session-start scans filter on `agent-relay` only, so the swap is the lock — and
unlike the file channel it is near-atomic (the Gitea API is the single source of truth,
no git-sync lag). **Never start work on an issue labelled `agent-working`** unless the
claim is stale: no activity (comments/commits referencing it) for **2 hours** → any
handler may take over (comment the takeover first).

**Handle — never act silently.** Whatever you do, you MUST post a comment reporting
the **conclusion *or* inconclusion** of your work (what you did + commit refs, or why
you couldn't and what's still needed), then:

- **Resolved** → remove `agent-working` (final label set: none of the relay labels) and
  **close** the issue.
- **Inconclusive / blocked** → swap `agent-working` → **`agent-blocked`**, and leave
  the issue **open** so it stays findable and isn't silently dropped.

Removing `agent-relay` at claim time is what stops a recurring poller from
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

**Labels:** `agent-relay` = unprocessed inbound message; `agent-working` = claimed, in
flight (the lock); `agent-blocked` = processed but unresolved, needs attention.

### Not the backlog tracker — keep relay labels separate

The `agent-relay` / `agent-working` / `agent-blocked` labels are **only** this relay channel. They are
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
