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
    tool-result item types (inspect the larger real sessions or write fixtures
    from them; handle unknown item types gracefully rather than erroring).
  - assistant-only: `"api"`, `"provider"`, `"model"`, `"stopReason"`,
    optional `"errorMessage"`, and
    `"usage": {"input","output","cacheRead","cacheWrite","totalTokens","cost":{…}}`.
  - `timestamp` (epoch millis, inside `message`).

Real sample data exists on this machine under `~/.pi/agent/sessions/` (5
project dirs, sessions from ~200 B to ~450 KB) — use it to *derive* committed
test fixtures, but tests must run against fixtures in the repo (mirroring how
other providers' tests do it), never against the live home directory.

### Touch-points (mirror any recently added provider, e.g. `qwen`)

- `crates/history-core/src/providers/pi.rs` — `detect()`, `get_base_path()`,
  `scan_projects()`, `load_sessions(path, exclude_sidechain)`,
  `load_messages(path)`, `search(query, limit)`; normalize into
  `ClaudeProject`/`ClaudeSession`/`ClaudeMessage` (map Pi usage → `TokenUsage`,
  thinking items → thinking content, preserve model/stopReason metadata).
- `crates/history-core/src/providers/mod.rs` — `pub mod pi;`,
  `ProviderId::Pi` (`"pi"`, display name `"Pi"`), `parse`/`as_str`/
  `display_name`, `detect_providers()`, `scan_all_projects()`,
  `load_sessions()`/`load_messages()` dispatch. (The sync daemon and WebUI
  server pick the provider up from this registry automatically.)
- Frontend: whatever the established per-provider registration is (provider
  icon/label metadata, `common.provider.pi` i18n key in **all 5 locales** —
  en, ko, ja, zh-CN, zh-TW; run `pnpm run generate:i18n-types` and
  `pnpm run i18n:validate`).

## Acceptance Criteria

- With a Pi session store present (fixtures or `~/.pi/agent/sessions`), the
  app's provider list includes **Pi**, and scanning shows one project per Pi
  session directory whose path is the real `cwd` from the session header (not
  the escaped directory name).
- A Pi project's session list shows each JSONL session with a human-usable
  summary (derived from the first user text message), a correct message count
  (only `type:"message"` records — `session`/`model_change`/
  `thinking_level_change` records are not counted as messages), and timestamps.
- Opening a Pi session renders user text and assistant text; assistant
  thinking blocks appear as thinking content; assistant messages expose model
  name and token usage (input/output/cache mapped into the normalized usage
  fields).
- A session whose assistant turn has `stopReason:"error"` and an
  `errorMessage` still loads without breaking the session (the error is
  surfaced or skipped gracefully, not a parse failure).
- `history_core::providers::ProviderId::parse("pi")` round-trips
  (`as_str() == "pi"`), and `scan_all_projects()` includes Pi projects — with
  Rust unit tests over committed fixtures proving scan/load/messages behavior.
- All existing repo gates stay green (lint, tsc, i18n:validate, vitest,
  cargo fmt/clippy/test) — no regression to the other providers.
