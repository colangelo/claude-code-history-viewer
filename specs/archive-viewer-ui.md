# Archive viewer UI: browse and search the hub archive from the viewer

## Description

The React viewer and headless WebUI read only local provider stores today; the
cross-machine archive (hub read API — `/v1/search`, `/v1/projects`,
`/v1/sessions`, `/v1/sessions/{id}/messages`, live on the tailnet, spec
`openspec/specs/archive-search-api/spec.md`) is reachable only with `curl`.
This feature gives the archive a first-class UI surface (Gitea issue #5).

**Architecture decision — direct hub access, no viewer-side proxy.** The
frontend calls the hub straight from the webview/browser via
`src/services/hubApi.ts` (fetch + `Authorization: Bearer`). Do NOT add Tauri
commands or `/api/*` axum routes for hub data: that would duplicate the hub
surface behind the command/route parity trap (bug class of #340/#355). The
one backend change is hub-side **CORS** so browser contexts may call it.

Work areas:

1. **Hub CORS** (`crates/hub/src/lib.rs::router`): add a permissive CORS
   layer (`tower-http` 0.6 `cors` feature, already used by src-tauri) — any
   origin, methods GET+OPTIONS at least, allow the `authorization` header,
   and EXPOSE `x-total-count` so `fetch` can read the paging total. Bearer
   auth still gates every read; CORS only removes the browser block. Keep
   clippy/rustfmt clean (workspace lints).

2. **Settings** — two optional fields on `UserSettings`
   (`crates/history-core/src/models/metadata.rs`, camelCase serde:
   `archiveHubUrl`, `archiveHubToken`; both `skip_serializing_if =
   "Option::is_none"`), mirrored in the TS `UserSettings` type and
   `settingsSlice`, persisted through the existing
   `load_user_metadata`/`update_user_settings` path (plaintext in
   `user-data.json` is accepted — tailnet-internal read token; keychain is a
   non-goal). Fill in the committed stub
   `src/components/SettingsManager/sections/ArchiveHubSection.tsx` (keep its
   exported props signature EXACTLY: `{ initialUrl?, initialToken?, onSave(url,
   token) }`) and register it in the settings manager UI.

3. **Hub client** — implement the committed stub `src/services/hubApi.ts`
   (keep every exported signature and type EXACTLY as committed; frozen evals
   compile against them and mock this module). Plain `fetch`; bearer header
   on all but `healthz`; query params only when set; `sessionMessages` reads
   `X-Total-Count` into `totalCount` and accepts a numeric hub id or a
   provider session UUID as `ref`. `hubMessageToClaudeMessage` maps a hub
   `MessageRow` to the viewer's `ClaudeMessage` union (type/role/uuid/
   timestamp/content; content is already the normalized content-array or
   string) so existing renderers work; unknown/other `message_type` values
   must degrade to a renderable message, never throw.

4. **Archive browser UI** — implement the committed stub
   `src/components/ArchiveBrowser/index.tsx` (keep `ArchiveBrowserProps` —
   `{ config: HubConfig }` — exactly). Three-pane flow in one component
   tree: archived projects (name + machine hostname provenance) → sessions of
   the selected project (summary, message count, last activity) → messages of
   the selected session rendered via `hubMessageToClaudeMessage` + existing
   message rendering, paged (page size 200 = hub max) with a load-more
   affordance driven by `totalCount`. A search input queries `/v1/search`
   and renders hits (snippet + project + machine); activating a hit opens
   that session's messages via its `session_id` UUID. Loading and error
   states must be visible, not silent. Wire the browser into the app as its
   own mode (e.g. header/sidebar toggle shown only when both
   `archiveHubUrl` and `archiveHubToken` are set) — the mode wiring is
   reviewer-verified, not eval-frozen, so keep it simple.

5. **i18n** — every new user-facing string through `t()`, keys added to ALL
   5 locales (en, ko, ja, zh-CN, zh-TW) + `pnpm run generate:i18n-types`;
   `pnpm run i18n:validate` is in the gate. a11y per repo checklist
   (aria-labels on icon buttons, label/id pairing in the settings section).

Non-goals (do NOT implement): viewer-side proxy commands/routes for hub data;
reading `~/.config/cchv/daemon.toml` from the viewer; merging archive entries
into the local `ProjectTree`/provider filter; offline caching; archive write
operations; keychain/secret storage; token-stats/analytics over archive data;
changes to `crates/sync-daemon`.

Eval mechanics: T1 (vitest, `crates/loop-evals/tests/<runId>.eval.test.tsx`)
renders the committed stubs from `src/` via the `@/` alias with
`@testing-library/react`, stubbing `globalThis.fetch` for hubApi criteria and
`vi.mock("@/services/hubApi", …)` for component criteria — no real network,
no server. Against the unmodified stubs every criterion fails at runtime
(stubs throw / render an empty div). T2 (Rust,
`crates/loop-evals/tests/<runId>_eval.rs`, nextest loop profile) drives
`hub::router` exactly like `crates/hub/tests/read_test.rs::spawn()` (pool
from `TEST_DATABASE_URL`, `hub::MIGRATOR`, unique machine per test) and, for
the settings criterion, `serde_json` round-trips through
`history_core::models::UserSettings` as a dynamic surface (`from_value` →
`to_value`, asserting key survival at runtime — do not reference new struct
fields directly). All T1 criteria in the single templated `.eval.test.tsx`
file, all T2 criteria in the single templated `_eval.rs` file.

## Acceptance Criteria

- (T2) `OPTIONS /v1/projects` on `hub::router` with `Origin: http://localhost:1420`, `Access-Control-Request-Method: GET`, `Access-Control-Request-Headers: authorization` returns a 2xx response whose `Access-Control-Allow-Origin` is present and whose allow-headers grant covers `authorization`.
- (T2) An authenticated `GET /v1/sessions/{id}/messages` sent with an `Origin` header returns `Access-Control-Allow-Origin` and an `Access-Control-Expose-Headers` grant covering `x-total-count`, and the `X-Total-Count` header itself.
- (T2) `serde_json::from_value::<UserSettings>` on `{"archiveHubUrl":"http://h:8787","archiveHubToken":"tok"}` followed by `to_value` yields JSON where both keys survive with those exact values, and a `UserSettings` deserialized from `{}` serializes WITHOUT either key.
- (T1) With `fetch` stubbed, `hubApi.listProjects({url:"http://hub:8787",token:"tok"})` issues exactly one GET to `http://hub:8787/v1/projects` with header `Authorization: Bearer tok` and resolves to the stub's JSON array.
- (T1) With `fetch` stubbed to return a JSON page and header `X-Total-Count: 120`, `hubApi.sessionMessages(cfg, "6741a288-41fb-4cce-8b2d-a027c391b4da", {limit: 100, offset: 0})` targets `/v1/sessions/6741a288-41fb-4cce-8b2d-a027c391b4da/messages` with `limit=100&offset=0` and resolves `{messages, totalCount: 120}`.
- (T1) With `fetch` stubbed, `hubApi.search(cfg, "needle fix", {project: "alpha"})` targets `/v1/search` with query params `q=needle fix` (URL-encoded) and `project=alpha` and resolves to the hits array; a non-2xx stub response makes it reject.
- (T1) `hubMessageToClaudeMessage` on an assistant row (`message_type:"assistant"`, `role:"assistant"`, uuid, timestamp, content `[{"type":"text","text":"hello archive"}]`) returns a message with `type === "assistant"` whose content carries the text, and on a row with `message_type: null` returns a renderable message object rather than throwing.
- (T1) `ArchiveBrowser` with `hubApi` mocked to two projects (distinct names and machine hostnames) renders both names and both hostnames after load.
- (T1) In `ArchiveBrowser` with `hubApi` mocked, selecting a project triggers `listSessions` filtered to that project and renders the mocked session summaries.
- (T1) In `ArchiveBrowser` with `hubApi.sessionMessages` mocked to return 200 messages then 50 (totalCount 250), opening a session renders message text and a load-more control; activating it requests `offset: 200` and appends the remaining messages, after which no load-more control remains.
- (T1) In `ArchiveBrowser` with `hubApi.search` mocked, submitting a query renders the hit snippet and its machine hostname, and activating the hit calls `sessionMessages` with that hit's `session_id`.
- (T1) `ArchiveHubSection` rendered with `initialUrl`/`initialToken` shows both values in labelled inputs; editing them and saving invokes `onSave` with the edited values.
