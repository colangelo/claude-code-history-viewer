# static-archive-webapp

## ADDED Requirements

### Requirement: Standalone static build

The project SHALL provide a build target that produces the archive browser as
plain static files (`dist-archive/`) with no Tauri, WebUI-server, or other
backend dependency of its own; every network call the bundle makes SHALL go to
the user-configured hub base URL.

#### Scenario: Build produces self-contained static output

- **WHEN** the archive web build recipe is run
- **THEN** `dist-archive/` contains an `index.html` plus hashed assets that can
  be served by any static file server with no rewrite rules or server-side
  code

#### Scenario: No backend adapter in the bundle

- **WHEN** the static bundle runs in a plain browser (no Tauri, no `/api`
  server)
- **THEN** it never calls the `api()` command adapter or `@tauri-apps/api`,
  and only issues requests to the configured hub's `/v1/*` endpoints

### Requirement: Connect screen with persisted hub config

The webapp SHALL show a connect screen when no hub configuration is stored,
accepting a hub base URL and bearer token, persisting them in `localStorage`,
and validating them against the hub before entering the browser.

#### Scenario: First visit prompts for connection

- **WHEN** the webapp loads and `localStorage` has no hub configuration
- **THEN** the connect screen is shown with URL and token fields, and the
  browser UI is not rendered

#### Scenario: Successful connect persists and enters the browser

- **WHEN** the user submits a URL/token pair and a probe request to the hub
  succeeds
- **THEN** the configuration is saved to `localStorage` and the archive
  browser is rendered with that configuration

#### Scenario: Failed connect surfaces the error

- **WHEN** the probe request fails (network error, 401, or non-2xx)
- **THEN** a visible error message is shown, nothing is persisted, and the
  user can correct the fields and retry

#### Scenario: Returning visit skips the connect screen

- **WHEN** the webapp loads and `localStorage` holds a hub configuration
- **THEN** the archive browser is rendered immediately, and a disconnect
  affordance lets the user clear the stored configuration and return to the
  connect screen

### Requirement: Full archive browsing reuse

The webapp SHALL reuse the existing `ArchiveBrowser` component unchanged in
behavior: project list, session list, message rendering, and full-text search
against the hub.

#### Scenario: Browsing works end to end

- **WHEN** a connected user selects a project and a session
- **THEN** messages render through the same renderers as the desktop Archive
  mode

### Requirement: Localized UI

All user-facing strings introduced by the webapp (connect screen, errors,
disconnect) SHALL exist in all 5 locales (en, ko, ja, zh-CN, zh-TW) and pass
`i18n:validate`.

#### Scenario: Locale validation passes

- **WHEN** `pnpm run i18n:validate` runs after the change
- **THEN** it reports all locales in sync with no duplicate keys
