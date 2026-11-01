## Why

The review of the distiller's protected routing (Gitea #84, finding #10, P1) showed that a client
session started outside a protected path (from the home folder, or from a coordinator repo) is
protected only by the content terms, and only by terms that survive into the prompt. The routing
check reads the prompt after two cuts: `_content_text` keeps 300 characters of a tool input and 500
of a thinking block, and `truncate` keeps a head and a tail of each session. So a session whose
working folder or file paths sit under a protected marker, or whose only client term falls in a
dropped middle, goes to the general backend.

**Revision 2 (2026-10-31)** follows the client-side second review of revision 1 (two separate
Claude reviews, merged as findings M1–M10, verdict "revise, then go"). Revision 1 claimed a `cwd`
signal the stored record cannot provide (M1), matched markers only on paths (M2), had no task for
the term half of #10 (M3), and routed on a different string than the one sent (M4). It also left
sidechains (M5), later days of the same session (M6) and the open-threads feed (M7) outside the
decision, matched only verbatim path shapes (M8), and failed open on some edges (M9).

## What Changes

- **The working folder is stored.** `ClaudeMessage` gains a nullable `cwd`, filled by the Claude
  provider from the source record, so the daemon's normalized `raw` carries it. The hub's
  `GET /v1/sessions/:id/messages` returns it. History is not re-ingested: the signal covers data
  ingested after both Macs run the new daemon, and the spec says so.
- **One flatten, one view.** A single flatten pass over the group's messages produces both the
  capped transcript lines and an uncapped routing view, so the prompt is a subset of the view by
  construction. Sidechain messages enter the view (not the transcript). The view reads each
  session's whole history, not just the day, so protection sticks to the session.
- **Markers everywhere, terms everywhere.** Path markers match the project path, every `cwd`, every
  tool-input value (decoded and normalized) and the full text, tool results included. Content terms
  match the full text. Path shapes are normalized before matching.
- **Send-time gate.** On the general route, the exact string about to be sent is checked last for
  every marker and term; any hit protects the group.
- **Fail closed, completely.** An unreadable session, a partial read, an unexpected content shape,
  a missing `cwd` key from an updated hub, or a routing file below its recorded floor protects the
  group, with a logged class. Matching stops at the first hit and never holds one large string.
- **The routing file's half is a task.** The client's manager adds the terms and markers #10 needs
  (an interim set was applied 2026-10-31), and #13's floor and fingerprint land before release.
  #10 closes only when both halves are in.
- **Release gates:** #12's origin filter (threads from protected entries offered only to protected
  calls) ships with or before this change; #14's hardened protected call is recommended to land
  with it; #18 (protected error paths log no derived text) is linked.

## Capabilities

### Modified Capabilities
- `journal-entries`: protected routing decides on the whole session and on the bytes sent.
- `archive-search-api`: session messages carry `cwd`.
- `archive-ingestion`: the stored normalized record keeps the working folder.

## Impact

- `crates/history-core` (`ClaudeMessage.cwd`, Claude provider), `crates/sync-daemon` (stores it via
  `raw`), `crates/hub/src/browse.rs` (returns it), `scripts/cchv-distill.py` (flatten, view,
  matcher, gate, fail-closed edges).
- Deploy order: hub, then the daemon on both Macs, then the distiller. Until a Mac runs the new
  daemon, its new messages have no `cwd`; the other signals still apply.
- More groups take the protected route (the point); measured by per-class counts after release.
- Second review by the client-side security reviewer: done for revision 1; revision 2 gets one
  re-test against M1–M4 before implementation.
