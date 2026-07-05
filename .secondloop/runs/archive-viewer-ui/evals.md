# Eval rubric — Archive viewer UI (browse and search the hub archive)

Feature: give the cross-machine hub archive (`crates/hub`, read API `/v1/search`,
`/v1/projects`, `/v1/sessions`, `/v1/sessions/{id}/messages`) a first-class UI
surface in the viewer (Gitea issue #5), reached with **direct hub access from
the frontend** (`src/services/hubApi.ts`, plain `fetch` + `Authorization:
Bearer`) — no viewer-side proxy commands/routes, per the command/route parity
bug class (#340/#355). The one backend change is hub-side CORS so a browser
context may call it.

Backend-observable criteria (hub CORS, `UserSettings` hub fields) are T2 and
live in `crates/loop-evals/tests/archive-viewer-ui_eval.rs` (3 tests, driving
`hub::router` exactly like `crates/hub/tests/read_test.rs`). Frontend-observable
criteria (hub client, archive browser component, settings section) are T1 and
live in the single file `crates/loop-evals/tests/archive-viewer-ui.eval.test.tsx`
(9 tests: `hubApi` exercised for real against a stubbed `global.fetch`, and
`ArchiveBrowser`/`ArchiveHubSection` exercised with individual `hubApi` methods
overridden via `vi.spyOn`). All 12 numbered criteria below map 1:1 to an
acceptance criterion in the feature spec; AC13-AC16 capture spec requirements
that are narrative rather than independently numbered and are rubric-only
(T3) — either because they're process/subjective, or because they're already
enforced by an existing repo-wide gate.

Confirmed: every T1/T2 eval fails against the current, unmodified stubs
(`hubApi.ts` throws `"hubApi: not implemented"` on every method,
`hubMessageToClaudeMessage` throws, `ArchiveBrowser`/`ArchiveHubSection` each
render only an empty `data-testid` div; `hub::router` has no CORS layer;
`UserSettings` has no `archiveHubUrl`/`archiveHubToken` fields) — 3/3 Rust
tests fail with real assertion failures (405 on the CORS preflight, missing
`Access-Control-Allow-Origin`, missing keys after a `serde_json` round trip),
and 9/9 vitest tests fail with real assertion failures (thrown "not
implemented" errors or missing DOM text/testid), not compile/syntax errors.

## Criteria

### AC1 — hub CORS preflight grants the `authorization` header (T2)
`OPTIONS /v1/projects` on `hub::router`, with `Origin: http://localhost:1420`,
`Access-Control-Request-Method: GET`, `Access-Control-Request-Headers:
authorization`, must return a 2xx whose `Access-Control-Allow-Origin` is
present and whose allow-headers grant covers `authorization`.
Eval: `ac1_cors_preflight_on_projects_grants_authorization_header`.

### AC2 — `X-Total-Count` is exposed to browser `fetch` via CORS (T2)
An authenticated `GET /v1/sessions/{id}/messages` sent with an `Origin`
header must return `Access-Control-Allow-Origin`, an
`Access-Control-Expose-Headers` grant covering `x-total-count`, and the
`X-Total-Count` header itself (asserted to be `"1"` for a session seeded with
one message).
Eval: `ac2_session_messages_exposes_x_total_count_via_cors`.

### AC3 — `UserSettings` gains `archiveHubUrl`/`archiveHubToken` (T2)
`serde_json::from_value::<UserSettings>` on
`{"archiveHubUrl":"http://h:8787","archiveHubToken":"tok"}` followed by
`to_value` must yield JSON where both keys survive with those exact values;
deserializing `{}` and re-serializing must omit both keys
(`skip_serializing_if = "Option::is_none"`). Uses the dynamic `Value`
round-trip surface only, per the loop's compile-against-unmodified-crate rule
— never references the new fields as struct fields directly.
Eval: `ac3_user_settings_archive_hub_fields_roundtrip_dynamically`.

### AC4 — `hubApi.listProjects` issues one authenticated GET (T1)
With `fetch` stubbed, `hubApi.listProjects({url, token})` issues exactly one
GET to `{url}/v1/projects` with header `Authorization: Bearer {token}` and
resolves to the stub's JSON array unchanged.
Eval: `hubApi > AC4: listProjects issues one authenticated GET to /v1/projects`.

### AC5 — `hubApi.sessionMessages` pages by ref and reads `X-Total-Count` (T1)
With `fetch` stubbed to return a JSON page and header `X-Total-Count: 120`,
`hubApi.sessionMessages(cfg, "<uuid>", {limit: 100, offset: 0})` must target
`/v1/sessions/<uuid>/messages` with query `limit=100&offset=0` and resolve
`{messages, totalCount: 120}`.
Eval: `hubApi > AC5: sessionMessages targets /v1/sessions/{ref}/messages and reads X-Total-Count`.

### AC6 — `hubApi.search` builds query params and rejects on failure (T1)
With `fetch` stubbed, `hubApi.search(cfg, "needle fix", {project: "alpha"})`
must target `/v1/search` with `q=needle fix` (decoded) and `project=alpha`,
resolving to the hits array; a non-2xx stub response must make the call
reject.
Eval: `hubApi > AC6: search targets /v1/search with q + filters, rejects on non-2xx`.

### AC7 — `hubMessageToClaudeMessage` maps rows and never throws (T1)
On an assistant row (`message_type: "assistant"`, `role: "assistant"`, text
content) the mapped message must have `type === "assistant"` and carry the
text in its content; on a row with `message_type: null` the function must
not throw and must return an object with a non-empty string `type`.
Eval: `hubApi > AC7: hubMessageToClaudeMessage maps a row and never throws on unknown type`.

### AC8 — `ArchiveBrowser` renders projects with name + machine hostname (T1)
With `hubApi.listProjects` mocked to two projects with distinct names and
machine hostnames, both names and both hostnames must appear after load.
Eval: `ArchiveBrowser > AC8: renders archived projects with name and machine hostname`.

### AC9 — selecting a project filters `listSessions` and renders sessions (T1)
Selecting a rendered project must call `hubApi.listSessions` with a `project`
filter equal to that project's `name` or `project_path`, and the mocked
session summaries must render.
Eval: `ArchiveBrowser > AC9: selecting a project filters listSessions and renders session summaries`.

### AC10 — message paging: load-more appends by `totalCount` (T1)
With `hubApi.sessionMessages` mocked to return 200 messages then 50
(`totalCount: 250`), opening a session must render message text and a
load-more control (`data-testid="archive-load-more"`); activating it must
request `offset: 200` and append the remaining messages, after which no
load-more control remains in the DOM.
Eval: `ArchiveBrowser > AC10: sessions page with load-more, appends remaining messages driven by totalCount`.

### AC11 — search renders hits and activating one opens its session (T1)
Submitting a query (via `data-testid="archive-search-input"`) with
`hubApi.search` mocked must render the hit's snippet and machine hostname;
activating the hit must call `hubApi.sessionMessages` with that hit's
`session_id` (the provider session UUID).
Eval: `ArchiveBrowser > AC11: search renders hit snippet/hostname and activating opens its session`.

### AC12 — `ArchiveHubSection` shows and saves hub settings (T1)
Rendered with `initialUrl`/`initialToken`, the section must show both values
in labelled inputs (`data-testid="archive-hub-url-input"` /
`"archive-hub-token-input"`, each associated with a `<label>` — checked via
the DOM `labels` property, independent of translated label text); editing
both and clicking `data-testid="archive-hub-save-button"` must invoke
`onSave(newUrl, newToken)`.
Eval: `ArchiveHubSection > AC12: shows initial values in labelled inputs and saves edits`.

### AC13 — loading and error states are visible, not silent (T3, rubric-only)
Every `hubApi` call site in `ArchiveBrowser` (project/session/message
loading, search) needs a visible loading indicator and a visible error
message on failure — not just a `console.error`. Not independently testable
without over-specifying DOM/copy the implementation is free to choose;
verify by reviewing the component for a loading branch and an error branch
around each `hubApi` call, and by manually forcing a rejected promise (e.g.
via the browser devtools network tab or a temporary bad token) and observing
that the UI shows something rather than staying blank.

### AC14 — archive mode entry point is simple and gated (T3, rubric-only)
The archive browser must be reachable as its own mode (header/sidebar
toggle) shown only when both `archiveHubUrl` and `archiveHubToken` are set in
settings. The spec explicitly marks this wiring "reviewer-verified, not
eval-frozen" — verify by inspecting the toggle's visibility condition and
confirming it renders `ArchiveBrowser` with a `HubConfig` built from those
two settings fields.

### AC15 — i18n coverage for every new user-facing string (T3, rubric-only)
Every new string introduced by `ArchiveHubSection` and `ArchiveBrowser` (labels,
placeholders, loading/error/empty-state text, the load-more control) must go
through `t()` with keys present in all 5 locales (en, ko, ja, zh-CN, zh-TW),
followed by `pnpm run generate:i18n-types`. Not part of the frozen eval
surface (the eval mocks `useTranslation` to a passthrough), but `pnpm run
i18n:validate` is in the release gate and will fail the build on drift —
verify by running it after implementation.

### AC16 — accessibility per the repo checklist (T3, rubric-only)
Icon-only buttons (e.g. a collapse/back control) need `aria-label`; the
`ArchiveHubSection` label/input pairs need `htmlFor`/`id` (`React.useId()`),
which AC12's `labels` check partially covers at the input level but does not
substitute for a full manual a11y pass (e.g. keyboard navigability of the
project/session/search-hit lists, focus handling when switching panes).
Verify by manual review against the "접근성 (a11y)" section of `CLAUDE.md`.
