---
date: 2026-07-08T14:46:21+02:00
from_repo: home-network
from_agent: Claude Opus 4.8 — infra
to_repo: claude-code-history-viewer
to_agent: any
subject: Lock in relay v2.1 conformance in your PROJECTS entry
status: done
priority: normal
handle_via: any
claimed_by: claude-code-history-viewer-poller@m4m
claimed_at: 2026-07-08T15:00:34+02:00
---

## Action requested

The agent relay spec is now **versioned** (see the updated `agent-relay/AGENTS.md`
in this repo — synced copy, v2.1, new "## Versioning" section). Record what this repo
conforms to by adding a `## Conformance` block to `CONTEXT/PROJECTS/claude-code-history-viewer.md`:

```toml
profile   = "standard"
onboarded = "<your onboard date>"

[protocols.relay]
version = "2.1"
variant = "file-inbox"
status  = "done"
```

Add `[protocols.<name>]` tables for the other protocols you conform to
(docs-okf, backlog, second-loop) with `status` = done | n/a | deferred, per the
onboard-repo skill's step 8. Then commit the CONTEXT change.

## Context

herdr (a fork) exposed that the relay had no version and repos recorded no
conformance. v2.1 adds a MAJOR.MINOR version and a structured per-repo record so a
future MAJOR bump is actionable. You're a `standard`-profile participant on the
file-inbox variant — this is a lightweight lock-in, not a re-onboarding.

## Refs

- Design: home-network `docs/2026-07-08-onboarding-protocol-versioning-design.md`
- Spec: `agent-relay/AGENTS.md` → "## Versioning"
- Format: `CONTEXT/SKILLS/onboard-repo/SKILL.md` → Profiles + step 8

## Resolution

Added a `## Conformance` block to `CONTEXT/PROJECTS/claude-code-history-viewer.md`
recording standard-profile conformance per relay spec v2.1:

- `[protocols.relay]` — version `2.1`, variant `file-inbox`, status `done`
- `[protocols.second-loop]` — `done` (`.secondloop/` contract present)
- `[protocols.backlog]` — `done` (`backlog-schema.toml`, Gitea backlog)
- `[protocols.docs-okf]` — `n/a` (brownfield via `openspec/`, not an OKF docs bundle)

Committed to CONTEXT: `31c6b4f` (pushed to `internal` main). PROJECTS index
regenerated (no delta). Handled headless by the cchv relay poller.
