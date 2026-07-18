# archive-journal-ui Specification (delta)

## ADDED Requirements

### Requirement: Journal/Browse tabs with journal as landing view

The archive browser SHALL present two top-level views behind a tab switcher —
**Journal** and **Browse** (the existing projects→sessions→messages panes) —
with Journal as the default on load. The global search bar SHALL remain
visible and functional in both views. The switcher MUST be keyboard-accessible
(real buttons with an accessible selected state).

#### Scenario: Journal greets on load

- **WHEN** the archive browser mounts with a valid hub connection
- **THEN** the Journal view is shown, and the Browse view is one tab-click away
  with its existing behavior unchanged

### Requirement: Day-timeline feed

The Journal view SHALL render journal entries from the hub's journal browse
endpoint as a reverse-chronological feed grouped by `entry_date`, each day
under a humanized header (relative labels like "Yesterday" for recent dates,
weekday + date otherwise). It SHALL provide: quick-nav pills for recent active
days, a date picker that jumps the feed to a chosen date, and a project filter.
Additional (older) days load on demand without losing scroll context. When a
range has no entries, the view MUST say so and note that older history may not
be distilled yet (absence of an entry is not absence of work).

#### Scenario: Yesterday at a glance

- **WHEN** entries exist for the previous logical day
- **THEN** the feed opens with that day's header (labelled as yesterday) and
  one card per project worked on that day

#### Scenario: Jump to a specific date

- **WHEN** the user picks a date in the date picker
- **THEN** the feed shows that date's entries (or the no-entries notice)

#### Scenario: Filter by project

- **WHEN** a project filter is selected
- **THEN** only that project's entries render, across all visible days

### Requirement: Rich entry cards with drill-down

Each entry card SHALL show at rest: project name, session count, generating
model, the headline, the summary clamped to ~2 lines, and topic chips.
Expanding a card SHALL reveal the full summary, the open questions (when any),
and links to the entry's sessions. Session labels (summary/message count)
SHALL be resolved lazily at expand time — no per-card session requests during
feed rendering. Activating a session link SHALL switch to the Browse view with
that session's messages open.

#### Scenario: Expand reveals the full story

- **WHEN** a card with open questions is expanded
- **THEN** the full summary, an open-questions list, and one labelled link per
  session id are shown

#### Scenario: Session link lands in Browse

- **WHEN** a session link is activated
- **THEN** the view switches to Browse and that session's messages load

### Requirement: Journal hits in search results

The client SHALL parse the `journal` block of the hub search response and
render journal hits as a visually distinct section above message hits, each
showing at least headline, date, and project. Activating a journal hit SHALL
open the Journal view positioned at that entry's date. Message-hit behavior
is unchanged.

#### Scenario: Distilled answer above raw hits

- **WHEN** a search term matches both a journal entry and archived messages
- **THEN** the journal hit renders in its own section above the message hits,
  and activating it shows that date in the Journal view

### Requirement: Unified archive type scale

Conversation content (user and assistant message text) SHALL render at a
14px-equivalent reading size with relaxed line height; tool-card headers SHALL
render at 13px medium weight with tool identifiers at 11px monospace —
subordinate to conversation content, never larger. Sidebar list rows SHALL be
13px with 11px metadata. The projects pane SHALL be ~240px wide and the
sessions pane ~320px. Timestamps in lists SHALL be humanized and counts
locale-formatted. The scale SHALL live in the shared renderer style tokens so
the desktop/WebUI viewer inherits the same hierarchy.

#### Scenario: Content outranks chrome

- **WHEN** a session with text messages and tool cards renders
- **THEN** message text is computed-styled larger than tool-card headers, and
  tool ids render smaller than their card headers

#### Scenario: Wider, readable lists

- **WHEN** the Browse view renders
- **THEN** the projects and sessions panes use the widened widths and 13px
  rows with humanized dates

### Requirement: Localized journal UI

All user-visible journal strings SHALL go through i18n keys present in all
five locales (en, ko, ja, zh-CN, zh-TW), passing `i18n:validate`.

#### Scenario: No hardcoded strings

- **WHEN** the journal view renders in any locale
- **THEN** every label resolves through a translation key (no raw English
  literals in the TSX)
