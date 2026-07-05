---
description: Check the agent relay (file inbox + Gitea agent-relay issues) and handle/report each
allowed-tools: Bash, Read, Edit, Glob, Write
---

You are the **claude-code-history-viewer** repo's agent (app). Check **both**
agent-relay channels for messages addressed to this repo and handle them. Protocol:
`agent-relay/AGENTS.md`. This may run headless (`claude -p`), so everything you need
is below.

Repo slug: `ac/claude-code-history-viewer`. Gitea API base:
`https://gitea.cat-bluegill.ts.net/api/v1`.
Get the token: `GITEA_TOKEN=$(printf 'protocol=https\nhost=gitea.cat-bluegill.ts.net\n\n' | git credential fill | awk -F= '/^password/{print $2}')`.

## 0. Which handler are you? — $ARGUMENTS

If the arguments above contain `--headless`, you are the **unattended poller**: you MUST
skip file messages with `handle_via: interactive` and issues labelled `relay-interactive`
— leave them completely untouched (don't claim, don't comment); they are reserved for a
session with the user present. Without `--headless` you are an attended session and may
handle everything (but leave `handle_via: poller` messages to the poller unless the user
says otherwise).

## 1. File inbox

```bash
find agent-relay/inbox -type f -name '*.md' -exec grep -l 'status: new' {} + 2>/dev/null || true
```

For each: read it, then **claim it BEFORE doing any work** — set `status: in-progress`,
add `claimed_by: <role>@<host>` and `claimed_at: $(date -Iseconds)`, commit + push
immediately. Skip messages already `in-progress` unless `claimed_at` is older than
2 hours (stale claim — take over, update the claim fields, note it in the Resolution).
Then do the work, and **archive it** — `git mv` to `agent-relay/archive/`,
set `status: done`, append a `## Resolution` section (what you did + commit refs). Reply
to the sender's inbox if a response is warranted. Commit + push to **`internal`**
(never to the public GitHub remotes — relay content stays off GitHub).

## 2. Gitea issues labelled `agent-relay`

```bash
curl -s -H "Authorization: token $GITEA_TOKEN" \
  "https://gitea.cat-bluegill.ts.net/api/v1/repos/ac/claude-code-history-viewer/issues?state=open&labels=agent-relay"
```

For each open issue: read it (+ its comments), then **claim it BEFORE doing any work** —
swap the label `agent-relay` → `agent-working` (resolve label IDs via
`GET .../repos/ac/claude-code-history-viewer/labels`, then `PUT .../issues/<N>/labels {"labels":[<agent-working id>]}`),
optionally commenting `claimed by <role>@<host>`. Skip issues already labelled
`agent-working` unless the claim is stale (no activity for 2+ hours — comment the
takeover first). Then do the work it asks.

**Then — ALWAYS report back. Never act silently** (an unreported issue is a lost one):

1. **Post a comment** stating the **conclusion OR inconclusion** — what you did + commit
   refs, or, if you couldn't finish, *why* and *what is still needed*:

   ```bash
   BASE="https://gitea.cat-bluegill.ts.net/api/v1"
   curl -s -H "Authorization: token $GITEA_TOKEN" -H "Content-Type: application/json" \
     -X POST "$BASE/repos/ac/claude-code-history-viewer/issues/<N>/comments" -d '{"body":"…"}'
   ```

2. **Update labels + state** (resolve label IDs first via
   `GET .../repos/ac/claude-code-history-viewer/labels`; use
   `PUT .../issues/<N>/labels {"labels":[<ids>]}` to set the final label set):
   - **Resolved** → set labels to `[]` (drops `agent-working`) and **close**:
     `PATCH .../issues/<N> {"state":"closed"}`.
   - **Inconclusive / blocked** → set labels to `[<agent-blocked id>]` (drops
     `agent-working`, adds `agent-blocked`) and leave it **open**.

Dropping `agent-relay` at claim time is what stops the poller from reprocessing the
same issue forever; `agent-blocked` keeps an unresolved one findable.

## 3. Report

If neither channel had anything new, say so plainly ("no new relay messages"). Otherwise
summarise, per message, what you concluded (or why it's now `agent-blocked`) — this
output is the audit trail when run headlessly.
