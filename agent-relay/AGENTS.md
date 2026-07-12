# Agent relay protocol

**Spec version 2.4** — versioned `MAJOR.MINOR` (see [Versioning](#versioning)).
Repos record the version they conform to in their
`CONTEXT/PROJECTS/<repo>.md` → `## Conformance` block.

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
| `home-network` | infra | `/Users/ac/_sync/ac-devops/_projects/Infra/home-network` | `nats · agent-relay/inbox/` | `ac/home-network` |
| `siai` | ci | `/Users/ac/_sync/ac-devops/_projects/AI/siai` | `agent-relay/inbox/` | `ac/siai` |
| `direction` | app | `/Users/ac/_sync/Carlo/Projects/direction` | `agent-relay/inbox/` | `ac/direction` |
| `macos-setup` | dev-env | `/Users/ac/_sync/dev/macos-setup` | `nats · agent-relay/inbox/` | `ac/macos-setup` |
| `second-loop` | loop | `/Users/ac/_sync/dev/second-loop` | `agent-relay/inbox/` | `ac/second-loop` |
| `claude-code-history-viewer` | app | `/Users/ac/_sync/dev/claude-code-history-viewer` | `agent-relay/inbox/` | `ac/claude-code-history-viewer` |
| `sergente` | agent | `/Users/ac/_sync/dev/sergente` | `agent-relay/inbox/` | `ac/sergente` |
| `herdr` | app | `/Users/ac/_sync/dev/herdr` | `issues-only` | `AC-forks/herdr` |
| `mozeidon-z` | app | `/Users/ac/_sync/ac-devops/_projects/AI/firefox-ai/mozeidon` | `nats` | `A-Layer/mozeidon-z` |

All repos are local checkouts under the same user, so a sender writes to the
recipient's path directly. Across machines, the inbox travels via Gitea (commit + push;
the recipient pulls).

The **Inbox** column is the participant's channel variant: `agent-relay/inbox/`
(file mailbox, the default), **`issues-only`** (fork / minimal-surface — no committed
inbox, reachable only via the issues channel; see below), **`nats`** (broker transport,
zero committed surface — see *NATS channel*), or `—` (not reachable). A `·`-separated
list means the repo **receives on every listed channel, primary first** — senders
default to the primary; any listed channel is acceptable. `home-network` and
`macos-setup` are deliberately multi-channel receive-all (nats-primary): infra is the
bootstrap/break-glass destination (a relay-transport bug report must be able to arrive
on an *older* transport), and macos-setup repairs the workstation NATS clients (that
ask can't require the broken client). The issues channel is additionally available for
every participant regardless of this column (it's the tracked-asks channel, not a
transport variant).

**Ownership**: the registry above and the cross-repo sync of this spec are
**home-network's (infra)** — like the poller. Other repos propose changes via a relay
message/issue to home-network; infra lands the canonical wording and syncs every copy.
(Unowned "keep in sync when editing" is exactly how drift starts at 6+ participants.)

## Repo ownership — a needed change elsewhere is a relay signal, not an edit

Each repo is owned by its own agent. Working in repo A and finding that repo B needs
a change is, by default, a **signal to send a relay message — not to edit repo B
yourself**. Unless the cross-repo edit is a conscious decision (the user directed it,
or a standard explicitly assigns you that surface):

1. **Look the target repo up** in the project catalog
   (`~/_sync/dev/CONTEXT/PROJECTS/index.md`).
2. **Not in the catalog** → **stop**; suggest onboarding it (the `onboard-repo`
   skill) rather than editing a repo the constellation doesn't know.
3. **In the catalog and a relay participant** (registry above) → **send a relay
   message** (file inbox or `agent-relay` issue) describing the ask, and let that
   repo's own agent land the change.
4. **In the catalog but not a relay participant** → surface it to the user; propose
   relay onboarding if the need looks recurring.

**Sanctioned exceptions** — cross-repo writes that *are* the protocol or an assigned
surface, not violations of it:

- Writing a message file into another repo's `agent-relay/inbox/` (plus that commit
  & push) — that *is* the relay.
- home-network (infra) syncing this spec + registry into every participant's copy.
- A surface a house standard explicitly assigns cross-repo to a role.

Rationale: the owner agent has the repo's context (conventions, in-flight work,
backlog) that a passing agent lacks; silent foreign edits are how conflicting
half-understandings land. The relay exists precisely so the ask travels instead of
the edit.

## Onboarding a participant

**A repo onboards BEFORE its agent sends its first relay message** — a sender without
an inbox has no return channel (learned 2026-07-03: cchv messaged second-loop with
nowhere to receive the reply; its scaffold had to be built after the fact).

1. Scaffold `agent-relay/{inbox,archive}/` (with `.gitkeep`s) and copy this spec file
   verbatim from any participant.
2. Add a session-start inbox pointer in the repo's `AGENTS.md`/`CLAUDE.md`. Do **not**
   add a per-repo `/check-relay` command — the handler is the single global
   **`check-relay` skill** (`~/_sync/dev/CONTEXT/SKILLS/check-relay/`, chezmoi-symlinked
   onto every Mac as a user-scope skill), which self-locates via the registry table in
   this spec. If the repo has a legacy `.claude/commands/check-relay.md`, **delete it**
   (per-repo copies drift — all 7 had diverged by 2026-07-06).
3. Ask **home-network (infra)** for a registry row (relay message or `agent-relay`
   issue); infra adds it and syncs all spec copies.

### Fork / minimal-surface participants

A repo that is a **fork of an active upstream** (onboard profile `fork`) keeps its
committed surface minimal — every added file becomes a patch that rides every
upstream rebase. Such a repo joins the relay **issues-only** (via the *Issues channel
(Gitea)* below, never a file inbox) and commits **no `agent-relay/` directory**:

- Register with **Inbox = `issues-only`** in the table above.
- A sender addressing a fork MUST use the **issues channel** (a Gitea issue in the
  fork's repo labelled `agent-relay`), never a file inbox.
- **Create the three relay labels** (`agent-relay`, `agent-working`, `agent-blocked`)
  on the fork's internal Gitea repo — these are the relay channel's labels, **not**
  backlog labels, so a fork's "backlog = N/A" does not create them.

The fork records `relay: variant = "issues-only"` in its PROJECTS `## Conformance`
block (see the onboard-repo skill).

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
| `handle_via` |  | which handler may claim it: `any` (default) / `interactive` / `poller`. `interactive` = **attended session only** — the background poller must skip it; set this when the user should see the exchange live (decisions, anything they'll ask the fg agent about). `poller` = background handler only — attended sessions leave it unless the user explicitly asks. |

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
2. **Claim — MANDATORY, before doing any work.** Eligibility comes first: respect
   `handle_via` — a headless/poller handler **never claims** a `handle_via: interactive`
   message (leave it untouched, don't even mark it), and an attended session leaves
   `handle_via: poller` ones to the poller unless the user asks. Then set `status: in-progress`, add
   `claimed_by: <role>@<host>` and `claimed_at: $(date -Iseconds)`, then **commit & push
   immediately** (before starting the actual work). This is the lock: all scans filter on
   `status: new`, so a claimed message is invisible to other handlers — instantly for
   sessions on the same machine (poller vs interactive share the checkout), after the
   next pull elsewhere. **Never start work on a message someone else has claimed** (see
   *Stale claims* below for the one exception).
3. **Handle** — when done, the **same party that claimed it** moves the file to
   `archive/`, sets `status: done`, and appends a `## Resolution` section (what was
   done + commit refs). Never leave handled work unarchived — a dangling `new` re-triggers the
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
- Should it only be handled with the user present? **Also label it `relay-interactive`**
  (the issues-channel analogue of `handle_via: interactive`): the poller's gate excludes
  it, so only an attended session will claim it.

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
  "https://gitea.cat-bluegill.ts.net/api/v1/repos/ac/<repo>/issues?state=open&labels=agent-relay" \
  | jq '[.[] | select(([.labels[]?.name] | index("relay-interactive")) | not)] | length')
[ "$n" -gt 0 ] && claude -p "/check-relay --headless" --allowedTools "Bash,Read,Edit"
```

The `--headless` argument tells `/check-relay` it is running unattended, so it must skip
`handle_via: interactive` files and `relay-interactive` issues (both jq filter above and
handler check exist — the gate keeps the poller from waking for nothing, the handler
check keeps a mixed batch safe).

`/loop` is in-session only and cloud Routines can't reach the tailnet Gitea, so use a
local launchd/cron job on an always-on tailnet host. Standing this up is **infra's**
(`home-network`) job. The `/check-relay` handler is the single global `check-relay`
skill (see *Onboarding a participant*), shared by every repo — no per-repo command.

**Labels:** `agent-relay` = unprocessed inbound message; `agent-working` = claimed, in
flight (the lock); `agent-blocked` = processed but unresolved, needs attention;
`relay-interactive` = attended-session-only (additive to `agent-relay`; the poller skips it).

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

## NATS channel — broker transport, zero committed surface (PILOT)

The third channel: the same message schema carried over the **NATS JetStream work
queue** on `bus.cat-bluegill.ts.net` instead of committed files. A `nats`-variant
participant commits **no `agent-relay/` directory at all** — participation is its
registry row here plus workstation creds. This is the variant for repos whose tree
is (partly) public: nothing about the constellation ever enters their history.
Design: home-network `docs/2026-07-08-agent-relay-nats-jetstream-design.md` ·
infra: `hosts/configs/proxmox1/bus.md` · pilot participant: `mozeidon-z`.

**Semantics (differences from the file channel):**

- **Delivery IS the claim.** The `RELAY` stream has work-queue retention: the server
  hands each message to exactly one handler; there is no status field to edit, no
  claim commit, no cross-machine pull-lag race. The lease is the consumer's `AckWait`
  (extended by working-acks during long runs); a crashed handler's message
  **redelivers automatically** (max 4 deliveries, then it alarms via ntfy).
- **No archive step.** `RELAY_AUDIT` (180d) is the durable trail: senders publish a
  full-copy `sent` event, handlers an `outcome` event. Never store the only copy of
  anything in `RELAY` itself — an acked work-queue message is gone.
- **`handle_via` is structural**, not a convention: `auto` messages go to subject
  `relay.msg.<repo>.auto` (bound by the always-on supervisor); `interactive` ones to
  `relay.msg.<repo>.interactive`, bound only by per-repo `interactive-<repo>`
  consumers that an **attended** session drains. A headless handler physically
  cannot receive them.

**Send** (any participant, to any registry repo):

```bash
~/_sync/ac-devops/_projects/Infra/home-network/tools/relay-send \
  --to <repo> --subject "<one line>" [--class auto|interactive] \
  [--priority low|normal|high] [--thread <msg-id>] [--body "<text>"]   # or body on stdin
```

Body = the same `## Action requested / ## Context / ## Refs` structure. `--from` is
derived from your cwd via the registry. Creds: `~/.config/agent-relay/env` (render
once with `just relay-env` in home-network; source `kv/agents/bus-relay` in OpenBao).

**Receive:**

- `auto` — handled by the **relay-supervisor** (launchd `dev.agent-relay.supervisor`
  on the always-on Macs; home-network `tools/relay-supervisor.py`): wakes on delivery
  (no polling), runs `claude -p` in the recipient repo's checkout, acks/naks/terms,
  audits. Repos are resolved from the registry table above — onboarding a repo needs
  no supervisor change.
- `interactive` — an attended session in repo `<repo>` drains its own consumer:

  ```bash
  nats --server "$NATS_URL" --user "$NATS_USER" --password "$NATS_PASSWORD" \
    consumer next RELAY interactive-<repo> --timeout 2s   # creds from ~/.config/agent-relay/env
  ```

**Routing rule:** send to the recipient's **primary** (leftmost) registry channel by
default; any channel the recipient lists is acceptable. A repo listing only `nats`
MUST be addressed via NATS (it has no file inbox). Any other participant MAY also be
addressed via NATS (the supervisor serves every registry repo); its file inbox and
the issues channel keep working unchanged. The issues channel remains the right
place for tracked, human-visible asks regardless of variant. Fallback is legitimate:
if the primary transport is down or your client is broken, use the recipient's next
listed channel — that is exactly why infra and macos-setup stay multi-channel.

## Versioning

This spec is versioned **`MAJOR.MINOR`**:

- **MINOR** — additive or clarifying; backward-compatible. A repo conforming to an
  earlier MINOR of the same MAJOR still conforms. Bump for new optional fields,
  clarifications, or new participant variants.
- **MAJOR** — breaking. Conforming repos must re-onboard the relay (re-read the spec,
  re-record their conformance). Bump for changed lifecycle semantics, renamed
  required fields, or removed channels.

Current: **2.4**. History — **2.0**: the pre-versioning mature spec (file inbox +
issues channel + poller + `handle_via`); **2.1**: adds this version field, the
`issues-only` registry variant, and the fork / minimal-surface participant path;
**2.2**: adds the *Repo ownership* norm (a needed change in another repo defaults
to a relay message, with the catalog-lookup procedure and sanctioned exceptions);
**2.3**: adds the *NATS channel* (broker transport on `bus`, delivery-as-claim,
`RELAY_AUDIT` trail, `relay-send`/supervisor tooling) and the `nats` registry
variant — pilot: `mozeidon-z`; **2.4**: multi-channel Inbox values (`·`-separated,
primary first) with the send-to-primary routing rule — `home-network` and
`macos-setup` flip to nats-primary receive-all (bootstrap/break-glass rationale
recorded at the registry).

Repos record which version they conform to in `CONTEXT/PROJECTS/<repo>.md` →
`## Conformance` (format defined in the onboard-repo skill).

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
