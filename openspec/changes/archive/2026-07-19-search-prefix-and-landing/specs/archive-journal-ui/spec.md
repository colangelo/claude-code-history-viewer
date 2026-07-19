# archive-journal-ui Specification (delta)

## MODIFIED Requirements

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

## ADDED Requirements

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
