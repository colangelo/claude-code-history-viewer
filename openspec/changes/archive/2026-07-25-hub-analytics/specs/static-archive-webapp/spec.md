## ADDED Requirements

### Requirement: Analytics view

The webapp SHALL offer an Analytics view alongside the existing Journal and
Browse views, presenting the connected hub's archive statistics: token and cost
totals over time split by model and provider, per-project work history, tool and
skill usage, and activity rhythm. The view MUST source every figure from the
hub's statistics endpoints — it MUST NOT recompute aggregates client-side from
fetched messages.

Scope selection MUST allow the whole archive or a single project identity, and
the view MUST support restricting figures to a date window.

#### Scenario: Analytics is reachable alongside Journal and Browse

- **WHEN** a user is connected to a hub
- **THEN** an Analytics view is selectable next to Journal and Browse, and selecting it presents archive statistics

#### Scenario: Figures come from the hub

- **WHEN** the Analytics view renders totals for a scope
- **THEN** those totals are the values returned by the hub's statistics endpoints for that scope

#### Scenario: Scope and window narrow the figures

- **WHEN** a user selects a single project identity and a date window
- **THEN** the presented statistics cover only that identity within that window

### Requirement: Analytics degrades against hubs without statistics

The Analytics view SHALL degrade gracefully when the connected hub predates the
statistics endpoints. A hub that responds `404` to a statistics request MUST
produce an explanatory message telling the user the hub needs upgrading, and
MUST NOT break the Journal or Browse views or leave the app in an error state.

#### Scenario: Older hub yields an explanatory message

- **WHEN** the connected hub responds `404` to a statistics request
- **THEN** the Analytics view shows a message that the hub does not support analytics and needs upgrading

#### Scenario: Other views keep working against an older hub

- **WHEN** the connected hub does not support statistics
- **THEN** Journal and Browse remain fully usable

### Requirement: Localized analytics UI

Every user-visible string in the Analytics view SHALL be translated across all
five supported locales, consistent with the rest of the webapp.

#### Scenario: Analytics strings resolve in every locale

- **WHEN** the app runs in any of the five supported locales
- **THEN** the Analytics view displays translated strings with no missing-key fallbacks
