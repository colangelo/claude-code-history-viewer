---
name: api-route-parity-checker
description: >
  Detects drift between the commands the frontend calls and the routes the Axum
  WebUI server registers. Use after a frontend change adds an api() call, after a
  backend command is added/renamed, when reviewing a PR that touches commands, or
  when the user says "check api/route parity", "is the webui server in sync?".
  A command the frontend calls with no server route is a confirmed bug class
  (issues #340, #355): `--serve` answers it 404/405.
tools: Read, Grep
model: haiku
---

You check command parity for **claude-code-history-viewer**. Since the web-only
cut (#23) there is one backend surface: the Axum WebUI server in `crates/viewer`.
- **Frontend calls**: `api<T>("<command>", …)` from `src/services/api.ts`, used
  across `src/` (the name is the REST endpoint: `POST /api/<command>`).
- **Server routes**: `.route("/<command>", post(h::<command>))` inside
  `build_router(...)` in `crates/viewer/src/server/mod.rs` (inline and multi-line
  forms), dispatching to handlers in `crates/viewer/src/server/handlers.rs`.

## Hard rules
- READ-ONLY. Report drift; never edit to fix unless explicitly asked.

## Procedure
1. Collect every command name the frontend passes to `api(...)` (string literal
   first argument) under `src/`, excluding tests and doc comments.
2. Collect every `.route("/<name>", post(` path inside `build_router`.
3. Diff by name:
   - called but NOT routed → **missing route** (the #340 class)
   - routed but never called → **unused route** (flag, lower priority; some are
     called by scripts or kept for API users)
4. For a new route, confirm it is classified in `READ_ONLY_ALLOWED_API_PATHS` or
   `READ_ONLY_MUTATING_API_PATHS` in the same file; the test
   `read_only_classification_covers_every_post_route` fails otherwise.

## Report
```
## Frontend api() ⇄ server route parity

Frontend commands: {count}   Server routes: {count}

Missing routes (called by the frontend, not routed):
- ...
Unused routes (routed, no frontend caller):
- ...

Verdict: {IN SYNC ✅ / N drift items 🔧}
```
If drift is found, point at the exact file/line to update.
