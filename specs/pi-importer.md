# Pi coding agent provider

## Description

Add a read-only **Pi** provider to `crates/history-core`, so Pi coding-agent
sessions (badlogic's `pi` / Pi SDK agent) are browsable in the viewer and
archivable by the sync daemon, exactly like the 29 existing providers.
Upstream tracks this as issues #359 (pi agent) and #279 (oh-my-pi); we build it
here first and will port it back upstream later, so **keep the implementation
self-contained**: one new provider module + the standard registry touch-points,
following the conventions of the most similar existing providers (`qwen.rs` for
per-cwd JSONL stores; `codex.rs` for rollout-style event logs).

Criteria 1–5 are backend-observable → **T2 (Rust integration-test) evals**;
criterion 6 is frontend-observable → **T1 (vitest) eval**. All repo gates
(lint, tsc, i18n:validate, vitest, cargo fmt/clippy/test) must stay green —
that is enforced by the gate, not restated as a criterion.

### On-disk format (verified against real local data, `"version":3`)

Store root: `~/.pi/agent/sessions/`. One subdirectory per working directory,
named as an escaped cwd (e.g. `--Users-ac-dev-herdr--` for
`/Users/ac/dev/herdr` — path separators become `-`, wrapped in leading/trailing
`--`). Do **not** decode the directory name to recover the cwd; every session
file's header record carries the exact `cwd`.

Each session is one JSONL file named `<ISO-timestamp>_<uuidv7>.jsonl`
(e.g. `2026-06-08T20-31-45-261Z_019ea8ef-246d-76ee-9e21-449fc7bfade9.jsonl`).
Records (one JSON object per line), all sharing `id`, `parentId`, `timestamp`
(ISO 8601) except the header:

- `{"type":"session","version":3,"id":"<uuid>","timestamp":"…","cwd":"/abs/path"}`
  — first line; `id` is the session id, `cwd` the project directory.
- `{"type":"model_change", …,"provider":"anthropic","modelId":"claude-opus-4-8"}`
- `{"type":"thinking_level_change", …,"thinkingLevel":"high"}`
- `{"type":"message", …,"message":{…}}` — the conversation. `message` has:
  - `role`: `"user"` or `"assistant"`.
  - `content`: array of items — `{"type":"text","text":…}`,
    `{"type":"thinking","thinking":…,"thinkingSignature":…}`, and tool-call /
    tool-result item types (inspect the larger real sessions under
    `~/.pi/agent/sessions/` or derive fixtures from them; handle unknown item
    types gracefully rather than erroring).
  - assistant-only: `"api"`, `"provider"`, `"model"`, `"stopReason"`,
    optional `"errorMessage"`, and
    `"usage": {"input","output","cacheRead","cacheWrite","totalTokens","cost":{…}}`.
  - `timestamp` (epoch millis, inside `message`).

Real sample data exists on this machine under `~/.pi/agent/sessions/` (5
project dirs, sessions from ~200 B to ~450 KB) — use it to *derive* fixtures,
but evals/tests must build their fixture store in a `tempfile::TempDir` (never
read the live home directory).

### Touch-points (mirror any recently added provider, e.g. `qwen`)

- `crates/history-core/src/providers/pi.rs` — `detect()`, `get_base_path()`,
  `scan_projects()`, `load_sessions(path, exclude_sidechain)`,
  `load_messages(path)`, `search(query, limit)`; normalize into
  `ClaudeProject`/`ClaudeSession`/`ClaudeMessage` (map Pi usage → `TokenUsage`,
  thinking items → thinking content, preserve model/stopReason metadata).
  Expose a base-dir-parameterized scan (like `continue_dev::scan_projects_in`)
  so tests/evals can point it at a fixture store.
- `crates/history-core/src/providers/mod.rs` — `pub mod pi;`,
  `ProviderId::Pi` (`"pi"`, display name `"Pi"`), `parse`/`as_str`/
  `display_name`, `detect_providers()`, `scan_all_projects()`,
  `load_sessions()`/`load_messages()` dispatch. (The sync daemon and WebUI
  server pick the provider up from this registry automatically.)
- Frontend: the established per-provider registration (provider id/label/badge
  metadata used by the project tree and provider filter) plus the
  `common.provider.pi` i18n key in **all 5 locales** (en, ko, ja, zh-CN,
  zh-TW; run `pnpm run generate:i18n-types` and `pnpm run i18n:validate`).

## Acceptance Criteria

- (T2) Scanning a Pi session store (fixture directory in the store's layout) yields one project per session subdirectory, and each project's real path is taken from the session header's `cwd` field — not decoded from the escaped directory name.
- (T2) Listing a Pi project's sessions yields, per JSONL file: a summary derived from the first user text message, a message count that counts only `type:"message"` records (header/`model_change`/`thinking_level_change` records excluded), and session timestamps.
- (T2) Loading a Pi session's messages maps user text and assistant text to normalized message content; assistant thinking items become thinking content; and assistant `model` + `usage` (input/output/cacheRead/cacheWrite) are mapped into the normalized message metadata and token-usage fields.
- (T2) A session whose assistant turn has `stopReason:"error"` and an `errorMessage` loads without a parse failure: the session and its other messages are returned, and the failed turn carries an error indication.
- (T2) `history_core::providers::ProviderId::parse("pi")` returns `Some`, the parsed id round-trips through `as_str()` to `"pi"` and `display_name()` to `"Pi"`, and the registry dispatch (`providers::load_sessions`/`providers::load_messages` called with that parsed id) routes to the Pi module instead of erroring.
- (T1) The frontend registers `"pi"` as a first-class provider: the provider id lookup used by the UI recognizes `"pi"`, its display label resolves through the `common.provider.pi` i18n key (present in all 5 locales), and a project tagged `provider:"pi"` renders with the Pi label — not the default provider's.
