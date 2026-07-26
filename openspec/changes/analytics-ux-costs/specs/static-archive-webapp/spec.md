# static-archive-webapp — delta for analytics-ux-costs

## ADDED Requirements

### Requirement: Search overlay is scoped to content views

The global search bar and its results overlay SHALL render only in the Journal
and Browse views. In the Analytics view neither renders; pressing `/` there
SHALL switch to the Journal view and focus the search input. Search state
(query text and fetched results) MUST survive switching to Analytics and back.

#### Scenario: Analytics view has no search surface

- **WHEN** a user switches to the Analytics view while search results are open
- **THEN** neither the search bar nor the results overlay is rendered over Analytics

#### Scenario: Slash from Analytics lands in searchable context

- **WHEN** a user presses `/` while in the Analytics view
- **THEN** the view switches to Journal and the search input receives focus

#### Scenario: Results survive the round-trip

- **WHEN** a user with open search results visits Analytics and returns to Journal
- **THEN** the previously fetched results are still displayed

### Requirement: Hit activation dismisses the results overlay

Activating a search hit — journal or message — SHALL dismiss the results
overlay so the destination view (Journal at the anchored day, or Browse at the
matched message) is immediately visible. The query text SHALL remain in the
input.

#### Scenario: Journal hit lands in the Journal feed

- **WHEN** a user activates a journal search hit
- **THEN** the results overlay closes and the Journal feed anchored at the hit's day is visible without scrolling past results

#### Scenario: Message hit lands in the session

- **WHEN** a user activates a message search hit
- **THEN** the results overlay closes and Browse shows the session at the matched message

### Requirement: Journal search results are height-capped

The journal-hits section of the results overlay SHALL be height-capped and
scrollable, matching the message-hits section, so results never push the
active view's content below the viewport.

#### Scenario: Many journal hits stay contained

- **WHEN** a search returns more journal hits than fit the cap
- **THEN** the journal-hits section scrolls within its cap instead of growing unbounded
