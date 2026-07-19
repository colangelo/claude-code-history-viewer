# static-archive-webapp Specification

## Purpose
TBD - created by archiving change static-archive-web. Update Purpose after archive.
## Requirements
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

The webapp SHALL attempt a same-origin auto-connect before showing any
prompt: on load with no stored hub configuration, it probes the page origin
(no token); if the probe succeeds (the host authenticates the request, e.g.
via trusted Tailscale identity headers), the archive browser is rendered
immediately with the page origin as the hub. If the probe fails, the webapp
SHALL show the connect screen, accepting a hub base URL and bearer token,
persisting them in `localStorage`, and validating them against the hub
before entering the browser. Auto-connected sessions persist nothing.

#### Scenario: Same-origin auto-connect skips the gate

- **WHEN** the webapp loads with no stored configuration and an
  unauthenticated probe to the page origin succeeds
- **THEN** the archive browser is rendered against the page origin without
  showing the connect screen, and nothing is written to `localStorage`

#### Scenario: First visit without host auth prompts for connection

- **WHEN** the webapp loads, `localStorage` has no hub configuration, and
  the same-origin probe fails
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

The webapp SHALL provide every user-facing string it introduces (connect
screen, errors, disconnect) in all 5 locales (en, ko, ja, zh-CN, zh-TW),
passing `i18n:validate`.

#### Scenario: Locale validation passes

- **WHEN** `pnpm run i18n:validate` runs after the change
- **THEN** it reports all locales in sync with no duplicate keys


### Requirement: Reader controls in the connected header

When connected, the webapp header SHALL identify the hub it is talking to
(host, with the full URL available on hover) and SHALL provide: a font-size
control stepping `--app-font-scale` between 0.8 and 1.4, persisted in
`localStorage` (`cchv.archiveWeb.fontScale`, default 1.1 — the shared type
scale is tuned for the dense desktop viewer, the webapp reads one step
larger) and re-applied on load; and a theme control cycling light → dark →
system, persisted through the shared storage adapter and re-applied on load.
Control labels SHALL be localized with accessible names.

#### Scenario: Font preference survives reload

- **WHEN** the user steps the font size up twice and reloads the page
- **THEN** text renders at the persisted scale without further interaction

#### Scenario: Theme preference survives reload

- **WHEN** the user switches to dark and reloads the page
- **THEN** the dark theme applies without further interaction

#### Scenario: Hub identity visible

- **WHEN** the webapp is connected (same-origin or manual)
- **THEN** the header shows the hub host it is connected to

### Requirement: Tokenless manual connect

The manual connect form SHALL accept an empty token: the probe then relies on
host-side authentication (e.g. Tailscale serve identity headers), and a
successful tokenless probe persists a valid empty-token config that restores
on the next visit. A failed probe still shows the error and persists nothing.

#### Scenario: Identity-authed hub without a token

- **WHEN** the user submits only a hub URL and the host vouches for the probe
- **THEN** the browser connects, and the stored config round-trips with an
  empty token on reload
