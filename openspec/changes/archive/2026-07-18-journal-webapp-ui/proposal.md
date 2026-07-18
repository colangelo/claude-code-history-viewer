# Proposal: journal-webapp-ui

## Why

The hub has served journal entries since cchv-v0.6.0, but no UI surfaces them —
answering "what did I work on yesterday / on date D" still means API calls or
message-level search. Issue #16. Separately, the archive browser's typography
is incoherent: tool-card headers inherit the default 16px while conversation
text renders at 12px and sidebar chrome at 12/10px, and the project/session
columns (192/256px) truncate constantly — user-reported 2026-07-15.

Design decided interactively 2026-07-15 (visual companion session, four
choices locked): day-timeline journal as the default archive view, rich
entry cards, journal-default tabs, and a unified type scale.

## What Changes

- `ArchiveBrowser` gains a two-tab header — **Journal** (default landing) |
  **Browse** (existing 3-pane) — shared by the static webapp and the
  desktop/WebUI archive mode. Search stays global above both tabs.
- New **Journal view**: reverse-chronological day feed from
  `GET /v1/journal/entries` (paginated, grouped client-side by `entry_date`,
  humanized day headers), date-pill quick nav + date picker + project filter,
  "load more" for older days.
- New **entry card**: rich at rest (project/session-count/model meta, headline,
  2-line clamped summary, topic chips); expand reveals full summary, open
  questions, and session links. Session labels resolve lazily on expand (one
  sessions-list call per card); clicking a session link switches to Browse
  with that session open.
- `hubApi.search()` parses the response's `journal` block (served since
  v0.6.0, currently ignored); search results render journal hits as a distinct
  section above message hits; activating one jumps to Journal at that date.
- **Typography/layout pass** in the shared renderer tokens (desktop/WebUI
  inherit): conversation text 14px/1.55, tool-card headers explicit 13px
  medium + 11px mono tool-ids, sidebar rows 13px / meta 11px, projects column
  240px, sessions 320px, humanized dates + formatted counts.
- New i18n keys (`settings.archiveHub.journal.*`) in all 5 locales + type
  regen.

## Capabilities

### New Capabilities

- `archive-journal-ui`: the webapp journal experience — tabs, day feed, entry
  cards, session drill-down, search journal section, and the unified archive
  type scale.

### Modified Capabilities

- (none — `static-archive-webapp`/`hub-static-hosting` requirements are
  unchanged; this is a client feature on the already-specified hub API.)

## Impact

- **Code**: `src/components/ArchiveBrowser/` (tabs, `JournalView.tsx`,
  `JournalEntryCard.tsx`), `src/services/hubApi.ts` (journal endpoints +
  search block), `src/components/renderers/styles.ts` + tool-card header
  (type scale), small date/number format util, i18n locales ×5.
- **No hub/Rust change** — API shipped in v0.6.0.
- **Desktop/WebUI viewer**: inherits both the tabs (archive mode) and the
  type-scale correction via the shared components.
- **Tests**: T1 vitest (feed grouping/labels, card expand + lazy session
  resolution, tab switch + session-link drill-down, search journal section,
  hubApi parsing); `i18n:validate`.
- **Version**: minor → `cchv-v0.7.0`.
