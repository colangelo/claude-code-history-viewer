# static-archive-webapp Specification (delta)

## MODIFIED Requirements

### Requirement: Reader controls in the connected header

When connected, the webapp header SHALL identify the hub it is talking to
(host, with the full URL available on hover) and SHALL provide: a font-size
control stepping `--app-font-scale` between 0.8 and 1.4, persisted in
`localStorage` (`cchv.archiveWeb.fontScale`, default 1.1) and re-applied on
load; and a theme control cycling light → dark → system, persisted through
the shared storage adapter and re-applied on load. Control labels SHALL be
localized with accessible names.

#### Scenario: Font preference survives reload

- **WHEN** the user steps the font size up twice and reloads the page
- **THEN** text renders at the persisted scale without further interaction

#### Scenario: Theme preference survives reload

- **WHEN** the user switches to dark and reloads the page
- **THEN** the dark theme applies without further interaction

#### Scenario: Hub identity visible

- **WHEN** the webapp is connected (same-origin or manual)
- **THEN** the header shows the hub host it is connected to
