# archive-journal-ui Specification

## Purpose

The webapp journal experience over the hub's journal entries: a Journal/Browse
tab switcher (Journal default), a day-grouped entry feed with rich cards and
lazy session drill-down, journal hits in search, and the unified archive type
scale. Client-only; shipped cchv-v0.7.0 (issue #16).
## Requirements
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
15px-equivalent reading size with relaxed line height; tool-card headers SHALL
render at 14px medium weight with tool identifiers at 12px monospace —
subordinate to conversation content, never larger. Journal entry headlines
SHALL render at 16px, day group headers at 15px semibold (never smaller than
the card summaries they group), card summaries and sidebar list rows at 14px,
and metadata/chips/pills at 12px — no user-visible text below 12px-equivalent.
The projects pane SHALL be ~240px wide and the sessions pane ~320px at desktop
widths. Long-form reading surfaces (journal feed, messages pane) SHALL
constrain content to a readable measure (~56rem max column) rather than the
full viewport width. Timestamps in lists SHALL be humanized and counts
locale-formatted. The scale SHALL live in the shared renderer style tokens so
the desktop/WebUI viewer inherits the same hierarchy, and every size SHALL
remain reactive to `--app-font-scale`.

#### Scenario: Content outranks chrome

- **WHEN** a session with text messages and tool cards renders
- **THEN** message text is computed-styled larger than tool-card headers, and
  tool ids render smaller than their card headers

#### Scenario: Readable measure on wide screens

- **WHEN** the journal feed or a session's messages render in a viewport
  ≥1280px wide
- **THEN** the content column is capped near 56rem instead of spanning the
  viewport

### Requirement: Responsive Browse layout

Below the `md` breakpoint the Browse view SHALL present a stacked drill-down —
exactly one level visible at a time (projects, else sessions once a project is
selected, else messages once a session is open) — with a visible localized
back control at each deeper level. All three panes SHALL remain simultaneously
visible at `md` and above. Opening a session from the Journal view or a search
hit on a narrow viewport MUST land on a visible messages view.

#### Scenario: Session link is readable on a phone

- **WHEN** a session is opened from a journal card at a 390px-wide viewport
- **THEN** the messages render full-width with a back control, and no pane is
  clipped off-screen

### Requirement: Search result affordances

Message-hit snippets SHALL render the search API's `<b>` highlight markers as
visually highlighted text (never as literal angle-bracket text and never via
raw-HTML injection); unpaired markers degrade to plain text. Each hit SHALL
show a humanized timestamp (raw on hover) alongside its project/machine meta.
The results area SHALL show a localized hit count and a control that
clears/dismisses the current results without clearing the query input. `/`
pressed outside an editable element SHALL focus the search input, and the
Journal|Browse tablist SHALL support ArrowLeft/ArrowRight navigation.

#### Scenario: Highlighted snippet

- **WHEN** a search response snippet contains `foo <b>bar</b> baz`
- **THEN** the rendered snippet shows "foo bar baz" with "bar" visually
  highlighted and no literal `<b>` text

#### Scenario: Dismiss results

- **WHEN** results are displayed and the clear control is activated
- **THEN** the results (message and journal sections) disappear while the
  query text remains editable in the input

#### Scenario: Keyboard affordances

- **WHEN** `/` is pressed with focus outside an input
- **THEN** the search input receives focus; and ArrowRight on a focused tab
  moves selection to the next tab


### Requirement: Localized journal UI

All user-visible journal strings SHALL go through i18n keys present in all
five locales (en, ko, ja, zh-CN, zh-TW), passing `i18n:validate`.

#### Scenario: No hardcoded strings

- **WHEN** the journal view renders in any locale
- **THEN** every label resolves through a translation key (no raw English
  literals in the TSX)

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

### Requirement: Search-hit landing

Activating a message hit that carries `position` SHALL open the session at
the page CONTAINING the matched message, visually highlight that message and
scroll it into view. When the loaded window does not start at the session's
beginning, a localized "load earlier" control SHALL extend the window upward
and the progress indicator SHALL show the loaded range
(`{from}–{to} of {total}`). Hits without `position` (older hubs) SHALL open
at page 1 with unchanged behavior.

#### Scenario: Mid-session hit is visible immediately

- **WHEN** a hit at position 420 of a 450-message session is activated with a
  200-message page size
- **THEN** the window 400–449 loads, the matched message renders highlighted
  in view, the progress reads a range, and "load earlier" can walk back to
  the session start
