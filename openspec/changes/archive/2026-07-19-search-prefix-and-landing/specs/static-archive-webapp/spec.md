# static-archive-webapp Specification (delta)

## ADDED Requirements

### Requirement: Tokenless manual connect

The manual connect form SHALL accept an empty token: the probe then relies on
host-side authentication (e.g. Tailscale serve identity headers), and a
successful tokenless probe persists a valid empty-token config that restores
on the next visit. A failed probe still shows the error and persists nothing.

#### Scenario: Identity-authed hub without a token

- **WHEN** the user submits only a hub URL and the host vouches for the probe
- **THEN** the browser connects, and the stored config round-trips with an
  empty token on reload
