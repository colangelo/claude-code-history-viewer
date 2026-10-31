## Why

The review of the distiller's protected routing (Gitea #84, finding #10, P1) showed that a client
session started outside a protected path (from the home folder, or from a coordinator repo) is
protected only by the content terms, and only by terms that survive into the prompt. The routing
check reads the prompt after two cuts: `_content_text` keeps 300 characters of a tool input and 500
of a thinking block, and `truncate` keeps a head and a tail of each session. So a session whose
working folder or file paths sit under a protected marker, or whose only client term falls in a
dropped middle, goes to the general backend.

## What Changes

- The hub's `GET /v1/sessions/:id/messages` returns each message's working folder (`cwd`, from the
  stored raw record) as a new nullable field. Nothing else in the response changes.
- The distiller decides the route from a **routing view** of the whole group, built before any
  truncation: the project path, every message's `cwd`, the full text of every tool input, and the
  full text of every message. A path marker in the project path, in any `cwd` or in any tool input
  protects the group; a content term anywhere in the full text protects it. The prompt check stays,
  so nothing that protects today stops protecting.
- If the routing view cannot be built for a group (a hub read fails), the group is protected
  (fail closed), as a missing routing file already is.
- Wider content terms are a change to the routing file, which lives outside this repo and is owned
  by the client's manager; this change only makes every term reach the full text.

## Capabilities

### Modified Capabilities
- `journal-entries`: the protected-routing requirement decides on the full session, not the prompt.
- `archive-search-api`: session messages carry `cwd`.

## Impact

- `crates/hub/src/browse.rs` (one column), `scripts/cchv-distill.py` (routing view, `is_protected`).
- Deploy order: hub first, then the distiller. A distiller run against an older hub sees no `cwd`
  and still routes on tool inputs and full text; it logs that `cwd` was unavailable.
- Cost: the routing view holds a group's full text in memory once; groups are one project-day.
- Second review by the client-side security reviewer before implementation (two-vendor rule does not apply: client
  work is Claude-only).
