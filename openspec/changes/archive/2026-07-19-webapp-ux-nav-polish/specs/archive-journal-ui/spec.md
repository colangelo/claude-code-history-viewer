# archive-journal-ui Specification (delta)

## ADDED Requirements

### Requirement: Routable archive state

When hash routing is enabled (the standalone webapp), the archive browser
SHALL express its primary state in `location.hash` — `#/journal`,
`#/journal/<YYYY-MM-DD>`, `#/browse`, `#/browse/session/<ref>`,
`#/search/<query>` — such that: loading a deep link restores that state
(including fetching the named session or running the named search); browser
back/forward re-apply prior states; and user navigation (tab switch, date
jump, session open, search submit) updates the hash without echo loops.
Embedded hosts (desktop/WebUI) SHALL leave the URL untouched.

#### Scenario: Session deep link

- **WHEN** the webapp loads with `#/browse/session/42`
- **THEN** the Browse view opens with session 42's messages loading

#### Scenario: Back returns to the journal date

- **WHEN** the user jumps to a date, opens a session, then presses Back twice
- **THEN** the view returns through Browse to the journal anchored at that date

### Requirement: Browse pane sync on indirect session open

Opening a session from a Journal card or a search hit SHALL, when the
accompanying project context matches exactly one known project, select that
project and load its session list without clearing the opened session. An
ambiguous match (same path on several machines/providers) SHALL sync nothing.

#### Scenario: Search hit lands with panes in agreement

- **WHEN** a search hit with full project context opens a session
- **THEN** the projects pane shows that project selected and the sessions pane
  lists its sessions while the messages render

### Requirement: Message gutter

The archived messages pane SHALL render a role label and a humanized
timestamp (raw timestamp on hover) at each turn boundary (role change),
localized, at metadata size — no unlabeled wall of messages.

#### Scenario: Turn boundaries are visible

- **WHEN** a session alternating user and assistant messages renders
- **THEN** each role change is preceded by a gutter row naming the role and
  its timestamp

### Requirement: Stable quick-nav pills

Quick-nav date pills SHALL derive from the accumulated union of entry dates
seen this session (newest first, capped), so selecting a date or filter never
collapses the pills to the filtered range.

#### Scenario: Pills survive a date jump

- **WHEN** the user jumps to one date via a pill
- **THEN** the other date pills remain available for further hops

### Requirement: Provider identity in Browse

Project rows SHALL carry a provider badge (localized provider label) so
same-named projects ingested by different providers are distinguishable.

#### Scenario: Twin rows distinguishable

- **WHEN** the same project path exists for two providers on one machine
- **THEN** the two rows show different provider badges

### Requirement: Archived tool-card state

Tool cards rendered in archived context SHALL NOT display live-execution
"Pending" states for absent inline results (archived results are sibling
rows); error and completed states render unchanged.

#### Scenario: No bogus Pending

- **WHEN** an archived tool_use row renders without an inline result
- **THEN** no "Pending" badge or placeholder appears

### Requirement: Readable project filter

The journal project filter SHALL label options by project basename (appending
the parent segment only when basenames collide), sorted by label, with the
full path available as the option tooltip. The empty state for a filtered
date SHALL offer an action that clears the date filter.

#### Scenario: Collision keeps context

- **WHEN** two projects share a basename
- **THEN** both options append their parent directory segment
