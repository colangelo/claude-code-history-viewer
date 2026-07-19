# archive-journal-ui Specification (delta)

## MODIFIED Requirements

### Requirement: Unified archive type scale

Conversation content (user and assistant message text) SHALL render at a
15px-equivalent reading size with relaxed line height; tool-card headers SHALL render
at 14px medium weight with tool identifiers at 12px monospace — subordinate to
conversation content, never larger. Journal entry headlines SHALL render at 16px, day
group headers at 15px semibold (never smaller than the card summaries they group), card
summaries and sidebar list rows at 14px, and metadata/chips/pills at 12px — no
user-visible text below 12px-equivalent. The projects pane SHALL be ~240px wide and
the sessions pane ~320px at desktop widths. Long-form reading surfaces (journal feed,
messages pane) SHALL constrain content to a readable measure (~56rem max column) rather
than the full viewport width. Timestamps in lists SHALL be humanized and counts
locale-formatted. The scale SHALL live in the shared renderer style tokens so the
desktop/WebUI viewer inherits the same hierarchy, and every size SHALL remain reactive
to `--app-font-scale`.

#### Scenario: Content outranks chrome

- **WHEN** a session with text messages and tool cards renders
- **THEN** message text is computed-styled larger than tool-card headers, and tool ids
  render smaller than their card headers

#### Scenario: Readable measure on wide screens

- **WHEN** the journal feed or a session's messages render in a viewport ≥1280px wide
- **THEN** the content column is capped near 56rem instead of spanning the viewport

## ADDED Requirements

### Requirement: Responsive Browse layout

Below the `md` breakpoint the Browse view SHALL present a stacked drill-down — exactly
one level visible at a time (projects, else sessions once a project is selected, else
messages once a session is open) — with a visible localized back control at each deeper
level. All three panes SHALL remain simultaneously visible at `md` and above. Opening a
session from the Journal view or a search hit on a narrow viewport MUST land on a
visible messages view.

#### Scenario: Session link is readable on a phone

- **WHEN** a session is opened from a journal card at a 390px-wide viewport
- **THEN** the messages render full-width with a back control, and no pane is clipped
  off-screen

### Requirement: Search result affordances

Message-hit snippets SHALL render the search API's `<b>` highlight markers as visually
highlighted text (never as literal angle-bracket text and never via raw-HTML
injection); unpaired markers degrade to plain text. The results area SHALL show a
localized hit count and a control that clears/dismisses the current results without
clearing the query input.

#### Scenario: Highlighted snippet

- **WHEN** a search response snippet contains `foo <b>bar</b> baz`
- **THEN** the rendered snippet shows "foo bar baz" with "bar" visually highlighted and
  no literal `<b>` text

#### Scenario: Dismiss results

- **WHEN** results are displayed and the clear control is activated
- **THEN** the results (message and journal sections) disappear while the query text
  remains editable in the input
