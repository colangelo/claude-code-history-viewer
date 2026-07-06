# static-archive-webapp — delta

## MODIFIED Requirements

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
