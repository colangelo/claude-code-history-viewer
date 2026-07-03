# Pi coding-agent provider — eval rubric

Spec: "Pi coding agent provider" — add a read-only Pi provider to
`crates/history-core`, following `qwen.rs` (per-cwd JSONL store) and
`codex.rs` (rollout-style event log) conventions. Upstream tracks this as
issues #359 (pi agent) and #279 (oh-my-pi).

All 6 acceptance criteria are covered by executable evals — there is no T3
(non-executable/rubric-only) criterion for this feature. Repo gates (lint,
tsc, i18n:validate, vitest, cargo fmt/clippy/test) are enforced separately by
the loop's `[gate]` stage, not restated here.

## Tier classification

| AC | Tier | Eval file | Test name |
|----|------|-----------|-----------|
| AC1 | T2 | `crates/history-core/tests/pi-importer_eval.rs` | `ac1_scan_yields_one_project_per_session_dir_with_cwd_from_header` |
| AC2 | T2 | `crates/history-core/tests/pi-importer_eval.rs` | `ac2_load_sessions_reports_summary_message_count_and_timestamps` |
| AC3 | T2 | `crates/history-core/tests/pi-importer_eval.rs` | `ac3_load_messages_maps_text_thinking_model_and_usage` |
| AC4 | T2 | `crates/history-core/tests/pi-importer_eval.rs` | `ac4_error_turn_loads_without_parse_failure_and_carries_error_indication` |
| AC5 | T2 | `crates/history-core/tests/pi-importer_eval.rs` | `ac5_provider_id_parses_round_trips_and_dispatches` |
| AC6 | T1 | `crates/history-core/tests/pi-importer.eval.test.tsx` | 5 `it()` blocks under `describe("Pi provider frontend registration (AC6)")` |

All 5 Rust evals were confirmed to **compile** clean against the unmodified
`history-core` crate (`cargo check -p history-core --test pi-importer_eval`),
pass `cargo fmt --check` and `cargo clippy --all-features -- -D warnings`,
and then **fail at runtime** (0 passed / 5 failed) when run against the
unmodified crate — none are vacuous. The 5 vitest evals were confirmed to run
(no import/compile errors under vitest's transpile-only mode) and **fail**
(0 passed / 5 failed) against the unmodified frontend.

---

## AC1 — T2 — project scan uses header `cwd`, not the decoded directory name

**Criterion**: Scanning a Pi session store (fixture directory in the store's
layout) yields one project per session subdirectory, and each project's real
path is taken from the session header's `cwd` field — not decoded from the
escaped directory name.

**Why T2**: purely a backend parsing/scanning concern (`providers::scan_all_projects`
+ `ClaudeProject.actual_path`); nothing here is observable from the UI layer
in isolation.

**Eval**: `ac1_scan_yields_one_project_per_session_dir_with_cwd_from_header`
builds two fixture session directories under `<TempDir>/.pi/agent/sessions/`,
each named with a **deliberately wrong** escaped-looking directory name (e.g.
`--Users-ac-dev-wrong-name-one--`), while each session's header JSON line
carries the real `cwd` (e.g. `/Users/ac/dev/real-project-one`). It points
`HOME` at the fixture via a `HomeGuard`, calls `providers::scan_all_projects()`,
filters projects where `provider == Some("pi")`, and asserts:
- exactly 2 Pi projects are found (one per session subdirectory), and
- their `actual_path` values are exactly the two *header* cwds — proving the
  implementation must read `cwd` from the header rather than attempt to
  decode/derive it from the (intentionally mismatched) directory name.

**Fails today because**: no `pi` provider exists in the registry, so
`scan_all_projects()` never tags any project with `provider: "pi"` — the
filtered list is empty and the length assertion (`2`) fails.

---

## AC2 — T2 — per-session summary, message count, and timestamps

**Criterion**: Listing a Pi project's sessions yields, per JSONL file: a
summary derived from the first user text message, a message count that
counts only `type:"message"` records (header/`model_change`/
`thinking_level_change` records excluded), and session timestamps.

**Why T2**: backend session-listing logic (`providers::load_sessions` →
`ClaudeSession.summary`/`.message_count`/`.first_message_time`/
`.last_message_time`).

**Eval**: `ac2_load_sessions_reports_summary_message_count_and_timestamps`
writes one session file containing, in order: a `session` header, a
`model_change` line, a `thinking_level_change` line, one user `message`
(text: "Please refactor the auth module"), and one assistant `message`
(thinking + text + usage). It scans to find the Pi project, calls
`providers::load_sessions(pi_provider, &project.path, false)`, and asserts:
- exactly 1 session is returned,
- `session.message_count == 2` (only the two `type:"message"` records —
  proving header/model_change/thinking_level_change are excluded from the
  count),
- `session.summary` is present and contains the first user message's text,
  and
- `first_message_time`/`last_message_time` are both non-empty.

**Fails today because**: `ProviderId::parse("pi")` returns `None` (no `pi`
variant registered), so the test panics immediately on `.expect(...)`.

---

## AC3 — T2 — message content/thinking/model/usage normalization

**Criterion**: Loading a Pi session's messages maps user text and assistant
text to normalized message content; assistant thinking items become thinking
content; and assistant `model` + `usage` (input/output/cacheRead/cacheWrite)
are mapped into the normalized message metadata and token-usage fields.

**Why T2**: pure parse/normalize logic (`providers::load_messages` →
`ClaudeMessage.content`/`.model`/`.usage: Option<TokenUsage>`).

**Eval**: `ac3_load_messages_maps_text_thinking_model_and_usage` writes a
session with one user text message and one assistant message whose `content`
array has a `thinking` item (`"Let me look at the auth module first."`) and a
`text` item (`"I refactored the auth module."`), plus `model:
"claude-opus-4-8"` and a `usage` object
(`input:1200, output:340, cacheRead:800, cacheWrite:0`). It calls
`providers::load_messages(pi_provider, &file_path)` and asserts:
- 2 normalized messages are returned (user + assistant),
- the user message's stringified `content` contains the original user text,
- the assistant message's `model == Some("claude-opus-4-8")`,
- the assistant message's stringified `content` contains both the thinking
  text and the reply text (proving the thinking item became thinking
  content, not silently dropped), and
- `usage.input_tokens/output_tokens/cache_read_input_tokens/
  cache_creation_input_tokens` equal `1200/340/800/0` respectively (Pi's
  `input`/`output`/`cacheRead`/`cacheWrite` mapped onto `TokenUsage`).

**Fails today because**: same as AC2 — `ProviderId::parse("pi")` returns
`None`, so the test panics on `.expect(...)` before reaching any assertion.

---

## AC4 — T2 — an errored assistant turn doesn't break parsing

**Criterion**: A session whose assistant turn has `stopReason:"error"` and an
`errorMessage` loads without a parse failure: the session and its other
messages are returned, and the failed turn carries an error indication.

**Why T2**: backend parse-robustness concern (`providers::load_sessions` /
`providers::load_messages` must not `Err(...)` or panic on this shape, and
`ClaudeSession.has_errors` / `ClaudeMessage.stop_reason` must reflect it).

**Eval**:
`ac4_error_turn_loads_without_parse_failure_and_carries_error_indication`
writes a session with: a user message, an assistant message with
`"stopReason":"error"` and `"errorMessage":"boom: connection reset"`, and a
**subsequent** user message. It asserts:
- `providers::load_sessions(...)` returns `Ok` with exactly 1 session, and
  that session's `has_errors == true`,
- `providers::load_messages(...)` returns `Ok` (no parse failure) with all 3
  messages present (proving the trailing message after the error turn is not
  dropped), and
- the assistant message's `stop_reason == Some("error")` and its serialized
  form contains the original error text — i.e., the failed turn visibly
  carries an error indication rather than being silently swallowed or
  crashing the parser.

**Fails today because**: `ProviderId::parse("pi")` returns `None` — same
panic-before-assertion failure mode as AC2/AC3.

---

## AC5 — T2 — `ProviderId` registration and dispatch routing

**Criterion**: `history_core::providers::ProviderId::parse("pi")` returns
`Some`, the parsed id round-trips through `as_str()` to `"pi"` and
`display_name()` to `"Pi"`, and the registry dispatch
(`providers::load_sessions`/`providers::load_messages` called with that
parsed id) routes to the Pi module instead of erroring.

**Why T2**: this is the exhaustive-`match` registry surface in
`crates/history-core/src/providers/mod.rs` — a pure backend concern.

**Eval**: `ac5_provider_id_parses_round_trips_and_dispatches` asserts
`ProviderId::parse("pi").expect(...)` returns `Some`, `.as_str() == "pi"`,
`.display_name() == "Pi"`; then, with a tiny fixture session directory in a
`TempDir`/`HOME`, calls `providers::load_sessions(id, ..., false)` and
`providers::load_messages(id, ...)` and asserts both return `Ok` with the
expected 1-element results — proving the dispatch `match` arms exist and
route to the Pi module rather than being an unmatched/missing-arm compile
error or a runtime "unsupported provider" error.

**Fails today because**: `ProviderId::parse("pi")` returns `None` today (no
`Pi` variant, so the whole test panics on the first `.expect(...)`); note
that referencing a not-yet-existing `ProviderId::Pi` variant directly would
be a *compile* error, so this eval deliberately stays on the dynamic
`parse`/dispatch surface, per the T2 "must compile against the unmodified
crate" rule.

---

## AC6 — T1 — frontend provider registration and label rendering

**Criterion**: The frontend registers `"pi"` as a first-class provider: the
provider id lookup used by the UI recognizes `"pi"`, its display label
resolves through the `common.provider.pi` i18n key (present in all 5
locales), and a project tagged `provider:"pi"` renders with the Pi label —
not the default provider's.

**Why T1**: entirely a frontend concern — `src/utils/providers.ts`'s
`PROVIDER_IDS`/`getProviderId`/`getProviderLabel`/`getProviderBadgeStyle` and
the `ProjectItem` component that renders the badge in the project tree /
provider filter.

**Eval**: `crates/history-core/tests/pi-importer.eval.test.tsx`, `describe("Pi
provider frontend registration (AC6)")`, 5 cases:
1. `PROVIDER_IDS` contains `"pi"` and `getProviderId("pi")` returns `"pi"`
   (not coerced to the default provider).
2. `getProviderLabel(translate, "pi")` resolves via the
   `common.provider.pi` key (asserted by injecting a `translate` fn that
   echoes `"<key>:<fallback>"` and checking the key prefix).
3. `common.provider.pi` exists as an own-property key in all 5 locale JSON
   files (`en`, `ko`, `ja`, `zh-CN`, `zh-TW`), imported directly.
4. `getProviderBadgeStyle("pi")` differs from `getProviderBadgeStyle("claude")`
   (Pi gets its own badge color, not the Claude default).
5. Rendering `<ProjectItem>` with `project.provider = "pi"` shows the Pi
   label and does **not** show `"Claude Code"` — i.e., a Pi-tagged project
   must not silently render as a Claude project.

**Fails today because**: `"pi"` is absent from `PROVIDER_IDS`/`ProviderId`/
`PROVIDER_TRANSLATIONS`/`PROVIDER_BADGE_STYLES` in `src/utils/providers.ts`,
so `getProviderId("pi")` currently falls back to `DEFAULT_PROVIDER_ID`
(`"claude"`), `common.provider.pi` doesn't exist in any locale file, and
`ProjectItem` renders a Pi-tagged project with the Claude badge/label
(confirmed by running the eval against the unmodified frontend: all 5 cases
fail, including the render case which finds `"Claude Code"` in the
document).
