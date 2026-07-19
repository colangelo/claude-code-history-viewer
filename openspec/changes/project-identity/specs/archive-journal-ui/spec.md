# archive-journal-ui Delta

## ADDED Requirements

### Requirement: Identity-grouped project surfaces

The Browse-view projects sidebar and the Journal-view project dropdown SHALL
group projects by `identity_key` (falling back to path for NULL): one entry
per identity, expandable/inspectable members with machine and provider
provenance, worktree members labeled. Selecting a grouped entry filters via
`project=identity:<key>`; ungrouped (fingerprint-less) entries keep their
exact-path filtering.

#### Scenario: Moved repo appears once

- **WHEN** the archive holds `~/dev/foo` and `~/projects/foo` under one identity
- **THEN** the dropdown and sidebar show a single `foo` entry whose selection covers both paths' history and journal

### Requirement: Basename display names with collision disambiguation

Grouped entries SHALL display the basename of the identity's most recently
active member path as the project name, with the full path(s) available on
inspection (e.g. title/tooltip). When two visible entries would share a
basename (e.g. a fork checked out elsewhere), each SHALL be disambiguated
with a distinguishing path suffix rendered as secondary text.

#### Scenario: Fork alongside original

- **WHEN** two different identities both basename `foo` are visible
- **THEN** both render as `foo` with a dimmed distinguishing directory suffix, and are never merged

### Requirement: Worktree visibility toggle

The webapp SHALL provide a persistent toggle (localStorage,
`cchv.archiveWeb.*` pattern, default: visible) controlling whether worktree
members are shown in grouped surfaces and included in identity-scoped queries
(`include_worktrees`). The toggle MUST NOT affect ungrouped path-identified
projects.

#### Scenario: Hiding worktrees

- **WHEN** the user disables worktree visibility and selects an identity with worktree members
- **THEN** worktree members disappear from the group display and their sessions/journal entries are excluded from results, and the preference survives reload

### Requirement: Alias suggestion and management affordances

Where the hub reports link suggestions, the UI SHALL surface a non-intrusive
affordance to link a suggested path to the identity (creating an alias) and
to unlink existing aliases from the identity's member inspection — both
explicit user actions with visible confirmation; the UI MUST NOT auto-link.

#### Scenario: Linking a dead path

- **WHEN** the hub suggests `~/old/foo` for identity `foo` and the user accepts
- **THEN** an alias is created, the group immediately includes the dead path's history, and an unlink affordance appears in the member list
