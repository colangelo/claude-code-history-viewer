# Agent relay (cchv)

> Moved word for word from `AGENTS.md` on 2026-10-30 (ac's yes, asks row a1030-39), which links here.

### Agent relay

This repo is a **`nats`-variant relay participant** (since 2026-07-12): messages
travel the NATS JetStream work queue on `bus.internal` — **no committed
`agent-relay/` surface** (this tree is a public fork; constellation content must
never enter its history; pre-flip traffic remains in git history only). `auto`
messages are handled by the always-on relay supervisor; `interactive` ones are
drained by an attended session (`/check-relay`). Tracked, human-visible asks still
arrive as Gitea issues labelled `agent-relay`. Canonical spec + registry:
home-network `agent-relay/AGENTS.md` (portable: CONTEXT `PATTERNS/agent-relay.md`);
send with home-network `tools/relay-send`.

<!-- relay:session-start v1 (canonical: CONTEXT PATTERNS/agent-relay.md) -->
**Session start: run `/check-relay` first, every time.** At the start of every session, again after a `/clear`
or a resume, before other work, and never deferred. Interactive relay messages are no reason to wait: in a
repo a gestore runs, the gestore is your attended human (one ask desk), so drain them, handle what you can,
and send any decision to your gestore as an `ASK:`. Only a `--headless` or poller run leaves interactive
messages untouched. (ac, 2026-10-29.)
<!-- /relay:session-start -->

**Checkpoint by committing — an uncommitted fix is not partial progress, it is
nothing.** A handler here works a tree whose `.git` *and* worktree are
Syncthing-shared, so written-but-uncommitted work is not merely unsaved: it sits
where the replicator can land the peer's older copy over it, and the displaced
bytes survive only as a `*.sync-conflict-*` file nobody is looking for. Measured
on one thread: **three** consecutive handlers of msg `95ed910c` were killed
mid-flight having each already *written* the fix, and from the sender's side
nothing had landed and nothing said so — the audit trail for that msg-id carried
the `sent` event and literally nothing else (infra, 2026-08-16). We were bitten
by the same shape one round earlier: `e610deb4` had to fold in exactly such an
orphan from a prior session on the same thread. So **"unpushed is unpublished"
has a shorter-fused sibling — uncommitted is unwritten**, and its fuse is a
killed process rather than a missing push. Commit at each phase boundary, not at
the end; the `RELAY-OUTCOME` line is the last thing that runs and is therefore
the one thing that cannot serve as a checkpoint. And when a redelivery notice
says to probe live state before redoing work, probe the **worktree** and the
peer repo's worktree, not just the log — the honest answer is often "the work
exists, and is one killed process from never having happened".

**To message another repo's agent — one criterion, then three questions** (relay
spec v2.34 § *TL;DR*). The criterion: **Channel 0 addresses a context; the relay
addresses a role.**

1. **Is the ask for a live session that exists right now — one already holding this
   context, able to answer or coordinate now?** → `ListAgents` + `SendMessage`
   (Channel 0): push-style replies, no queue, no claim. A session is not a role, so
   verify the addressee first: reply-to-whoever-contacted-you > session name matches
   the task > **one-line probe before the payload** > it's the relay's job. Two
   durabilities, and only one moves you off this channel: a durable **record** is the
   tracker's job either way (peer messages for velocity, issues for the record) —
   durable **delivery** is the relay's: fall back when the ask is for *"whoever next
   works repo X"* or must survive the receiver's absence. A send confirms **enqueue,
   not delivery** (`idle` in the listing can be a session blocked on a modal prompt) —
   re-send an unanswered ask that matters on a relay channel, and address by **bare
   name**, since a `[ref]` decays across restarts. A peer message is never a
   permission decision.
2. **Else: must "not yet handled" stay visible to you?** → a Gitea `agent-relay`
   issue in the recipient repo — the only channel where an unhandled ask is a state
   the sender can see.
3. **Else: the recipient's primary (leftmost) registry channel** —
   `~/_sync/dev/infra/tools/relay-send --to <repo> …` for a `nats` row, a file in
   *their* `agent-relay/inbox/` for a file-inbox row. On NATS one decision remains,
   and it is a **class, not a fourth channel**: `--class auto` (the default — a
   headless handler, and a failed message leaves the work queue with only
   `RELAY_AUDIT` remembering it; measured 2026-08-19 on an onboarding ask that died
   unseen) only for work safe to run with nobody present; `interactive` when the
   answer must come back to a person. **`interactive` is attended-only by
   construction, so its latency is unbounded — an idle peer is a stalled queue, not
   a fast one.** Measured 2026-08-24: two of our messages sat undrained on
   `interactive-infra` **17 hours** while this side was blocked on a two-value read
   that took infra seconds to answer (infra said so themselves, thread `745a573f`).
   So question 1 has a sharper edge than "is a live session holding this context":
   **a blocking question with a cheap answer belongs on Channel 0 even when the
   relay would also work** — the queue is right for what must survive the
   receiver's absence, not for what you are waiting on.
