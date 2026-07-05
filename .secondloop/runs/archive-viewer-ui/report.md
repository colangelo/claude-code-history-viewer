# Run report: archive-viewer-ui

**Repo:** /Users/ac/_sync/dev/claude-code-history-viewer
**Spec:** specs/archive-viewer-ui.md
**Status:** needs-human
**Started:** 2026-07-05T13:56:36.560Z  **Finished:** 2026-07-05T15:14:37.523Z

**Claude cost (counterfactual API value, billed to subscription):** $36.2150

**Error:** Review rounds exhausted without approval.

## Eval plan

| Criterion | Tier | Text |
|---|---|---|
| AC1 | T2 | (T2) `OPTIONS /v1/projects` on `hub::router` with `Origin: http://localhost:1420`, `Access-Control-Request-Method: GET`, `Access-Control-Request-Headers: authorization` returns a 2xx response whose `Access-Control-Allow-Origin` is present and whose allow-headers grant covers `authorization`. |
| AC2 | T2 | (T2) An authenticated `GET /v1/sessions/{id}/messages` sent with an `Origin` header returns `Access-Control-Allow-Origin` and an `Access-Control-Expose-Headers` grant covering `x-total-count`, and the `X-Total-Count` header itself. |
| AC3 | T2 | (T2) `serde_json::from_value::<UserSettings>` on `{"archiveHubUrl":"http://h:8787","archiveHubToken":"tok"}` followed by `to_value` yields JSON where both keys survive with those exact values, and a `UserSettings` deserialized from `{}` serializes WITHOUT either key. |
| AC4 | T1 | (T1) With `fetch` stubbed, `hubApi.listProjects({url:"http://hub:8787",token:"tok"})` issues exactly one GET to `http://hub:8787/v1/projects` with header `Authorization: Bearer tok` and resolves to the stub's JSON array. |
| AC5 | T1 | (T1) With `fetch` stubbed to return a JSON page and header `X-Total-Count: 120`, `hubApi.sessionMessages(cfg, "6741a288-41fb-4cce-8b2d-a027c391b4da", {limit: 100, offset: 0})` targets `/v1/sessions/6741a288-41fb-4cce-8b2d-a027c391b4da/messages` with `limit=100&offset=0` and resolves `{messages, totalCount: 120}`. |
| AC6 | T1 | (T1) With `fetch` stubbed, `hubApi.search(cfg, "needle fix", {project: "alpha"})` targets `/v1/search` with query params `q=needle fix` (URL-encoded) and `project=alpha` and resolves to the hits array; a non-2xx stub response makes it reject. |
| AC7 | T1 | (T1) `hubMessageToClaudeMessage` on an assistant row (`message_type:"assistant"`, `role:"assistant"`, uuid, timestamp, content `[{"type":"text","text":"hello archive"}]`) returns a message with `type === "assistant"` whose content carries the text, and on a row with `message_type: null` returns a renderable message object rather than throwing. |
| AC8 | T1 | (T1) `ArchiveBrowser` with `hubApi` mocked to two projects (distinct names and machine hostnames) renders both names and both hostnames after load. |
| AC9 | T1 | (T1) In `ArchiveBrowser` with `hubApi` mocked, selecting a project triggers `listSessions` filtered to that project and renders the mocked session summaries. |
| AC10 | T1 | (T1) In `ArchiveBrowser` with `hubApi.sessionMessages` mocked to return 200 messages then 50 (totalCount 250), opening a session renders message text and a load-more control; activating it requests `offset: 200` and appends the remaining messages, after which no load-more control remains. |
| AC11 | T1 | (T1) In `ArchiveBrowser` with `hubApi.search` mocked, submitting a query renders the hit snippet and its machine hostname, and activating the hit calls `sessionMessages` with that hit's `session_id`. |
| AC12 | T1 | (T1) `ArchiveHubSection` rendered with `initialUrl`/`initialToken` shows both values in labelled inputs; editing them and saving invokes `onSave` with the edited values. |
| AC13 | T3 | Loading/error-state visibility is a qualitative UX requirement without a specific DOM/copy contract given in the spec — not independently executable without over-constraining the implementation; captured as a rubric review item instead. |
| AC14 | T3 | Mode wiring (header/sidebar toggle gated on settings) is explicitly called out in the spec as 'reviewer-verified, not eval-frozen' — deliberately excluded from the frozen eval surface. |
| AC15 | T3 | i18n key coverage across 5 locales is already enforced by the repo-wide pnpm run i18n:validate gate (not part of the frozen eval's mocked-translation surface) — a rubric/process item, not a new executable criterion. |
| AC16 | T3 | Full accessibility pass (aria-labels, keyboard nav across panes) beyond the input/label pairing already covered mechanically in AC12 is a subjective manual-review item per the repo's a11y checklist. |

## Review rounds

### Round 1 — changes requested

- **blocker** `scripts/i18n-config.mjs`: The new archiveHub browser strings are added under archive.json, but archive is not in the configured i18n namespaces, so generate:i18n-types and i18n:validate do not cover them. src/i18n/types.generated.ts confirms only the new settings.* keys were generated. AC15 is not satisfied unless these keys are moved to a validated namespace or the archive namespace is added and types regenerated.
- **major** `src/components/ArchiveBrowser/index.tsx`: Selecting a project filters listSessions only by project name/path. In a cross-machine archive, two machines/providers can have the same project name/path, so selecting one project can show sessions from another machine. Include the selected project's machine_hostname and provider filters when loading sessions.
- **major** `src/components/ArchiveBrowser/index.tsx`: Session, message, and search requests have no cancellation or request identity guard. Rapid project/session/search changes can allow a slower stale response to overwrite the current selection's state and show the wrong archive data.
- **major** `src/components/ArchiveBrowser/index.tsx`: Messages are mapped with hubMessageToClaudeMessage but then collapsed through extractClaudeMessageContent before rendering. That drops normalized content-array blocks such as tool use/results/thinking, so archived messages are not rendered through the existing full message rendering path required by the spec.
### Round 2 — changes requested

- **blocker** `crates/hub/src/lib.rs`: CORS uses `.allow_headers(Any)`, which tower-http serializes as `Access-Control-Allow-Headers: *`. Browser CORS does not wildcard the non-wildcard `Authorization` request header, so direct frontend calls with `Authorization: Bearer ...` can still fail preflight. The spec explicitly requires allowing the `authorization` header; list `AUTHORIZATION` or mirror request headers.
- **blocker** `src/layouts/Header/Header.tsx`: The archive browser is wired as a modal (`openModal("archiveHubBrowser")`) with `isActive={false}`, not as its own app mode. AC14 requires the archive browser to be reachable as its own mode/header/sidebar toggle and rendered with a `HubConfig` derived from settings. Add a real archive-hub view mode instead of an always-inactive modal launcher.
### Round 3 — changes requested

- **major** `src/components/ArchiveBrowser/index.tsx`: `handleLoadMore` has no in-handler guard for an already in-flight page request. A rapid double-submit can call `sessionMessages` twice with the same offset before React applies the disabled state, appending duplicate messages. Return early when `isLoadingMessages` is true or track load-more in flight separately.
- **major** `src/components/mobile/BottomTabBar.tsx`: Archive Hub mode is only wired into the desktop header. Mobile navigation has no gated Archive Hub entry and `AppLayout`'s mobile switch has no `archiveHub` case, so the standalone archive browser is unreachable on mobile when hub settings are configured.
- **minor** `src/components/ArchiveBrowser/index.tsx`: The load-more button becomes spinner-only while loading and has no accessible label/text alternative for that icon-only state. Add an `aria-label` or keep screen-reader text in the loading branch.

## Deterministic gate


## Browser verification


## Commits

- f298d40 frozen evals
- 3e718bd implement
- 90e45cc fix round 1
- c559916 fix round 2
