# Run report: pi-importer

**Repo:** /Users/ac/_sync/dev/claude-code-history-viewer
**Spec:** specs/pi-importer.md
**Status:** needs-human
**Started:** 2026-07-03T13:17:26.409Z  **Finished:** 2026-07-03T14:28:30.656Z

**Claude cost (counterfactual API value, billed to subscription):** $21.5192

**Error:** Review rounds exhausted without approval.

## Eval plan

| Criterion | Tier | Text |
|---|---|---|
| AC1 | T2 | (T2) Scanning a Pi session store (fixture directory in the store's layout) yields one project per session subdirectory, and each project's real path is taken from the session header's `cwd` field — not decoded from the escaped directory name. |
| AC2 | T2 | (T2) Listing a Pi project's sessions yields, per JSONL file: a summary derived from the first user text message, a message count that counts only `type:"message"` records (header/`model_change`/`thinking_level_change` records excluded), and session timestamps. |
| AC3 | T2 | (T2) Loading a Pi session's messages maps user text and assistant text to normalized message content; assistant thinking items become thinking content; and assistant `model` + `usage` (input/output/cacheRead/cacheWrite) are mapped into the normalized message metadata and token-usage fields. |
| AC4 | T2 | (T2) A session whose assistant turn has `stopReason:"error"` and an `errorMessage` loads without a parse failure: the session and its other messages are returned, and the failed turn carries an error indication. |
| AC5 | T2 | (T2) `history_core::providers::ProviderId::parse("pi")` returns `Some`, the parsed id round-trips through `as_str()` to `"pi"` and `display_name()` to `"Pi"`, and the registry dispatch (`providers::load_sessions`/`providers::load_messages` called with that parsed id) routes to the Pi module instead of erroring. |
| AC6 | T1 | (T1) The frontend registers `"pi"` as a first-class provider: the provider id lookup used by the UI recognizes `"pi"`, its display label resolves through the `common.provider.pi` i18n key (present in all 5 locales), and a project tagged `provider:"pi"` renders with the Pi label — not the default provider's. |

## Review rounds

### Round 1 — changes requested

- **blocker** `crates/history-core/src/providers/pi.rs`: `load_messages()` rejects fixture/session paths unless they are under the current user's `~/.pi/agent/sessions` via `validate_under_base()`. The spec requires fixture stores in `TempDir` and a base-dir-parameterized scan/load path so tests and evals can point at non-home stores; registry dispatch AC5 calls `providers::load_messages(id, &file_path)` on a temp fixture path and this implementation returns `Err("Pi sessions path not found"/outside root)` instead of loading the message.
- **major** `crates/history-core/src/providers/pi.rs`: Pi message timestamps are not mapped from the nested `message.timestamp` epoch-millis field. `convert_record()` always uses the outer record `timestamp`, but the spec calls out `timestamp` inside `message`; real Pi data and normalized message ordering/metadata should preserve the actual message timestamp when present.
- **minor** `crates/history-core/src/providers/pi.rs`: The required base-dir-parameterized scan API is missing. The spec explicitly asks to expose a scan like `continue_dev::scan_projects_in` so tests/evals can target a fixture store directly; only `scan_projects()` using `dirs::home_dir()` is implemented.
### Round 2 — changes requested

- **major** `crates/history-core/src/providers/pi.rs`: `load_sessions` and `load_messages` trust caller-supplied paths and only reject symlinks, so `provider:"pi"` can enumerate/parse arbitrary directories or JSONL-shaped files outside `~/.pi/agent/sessions`. That is broader than the specified Pi store and unlike the similar providers' base-path validation. Validate canonical paths under the resolved Pi sessions root; the eval TempDir flow can still work by setting `HOME` to the fixture root.
### Round 3 — changes requested

- **major** `src-tauri/src/commands/session/mod.rs`: The WebUI session-path allowlist was not updated for Pi. After a session loads, the frontend calls `get_session_subagents`; in `webui-server` builds that handler rejects `~/.pi/agent/sessions/...` paths as outside the allowed provider directories, causing a user-visible failure when opening Pi sessions. Add `providers::pi::get_base_path()` to `is_safe_session_path`'s allowed roots, or avoid subagent fetches for non-Claude providers.

## Deterministic gate


## Browser verification


## Commits

- 5822439 frozen evals
- 3803cf9 implement
- 5ccc0c2 fix round 1
- 2172205 fix round 2
