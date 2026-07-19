# Proposal: webapp-ux-nav-polish

## Why

Phases 4–5 of the 2026-07-19 UX audit (Gitea #18; phases 1–3 shipped as
cchv-v0.8.0): the webapp still has no URL state (refresh resets everything,
nothing shareable, dead back button), Browse panes desync when a session is
opened from Journal/search, messages carry no role/timestamp, quick-nav pills
vanish once a date filter is set, identical project rows are indistinguishable
(no provider), there is no theme control, archived tool cards show misleading
live-state "Pending" chips, and the project filter is a wall of absolute paths.

## What Changes

- **Hash routing** (webapp-only via `enableHashRoutes` — embedded desktop/WebUI
  leave the URL alone): `#/journal[/date]`, `#/browse[/session/<ref>]`,
  `#/search/<q>`; deep links load their target on mount; back/forward apply
  routes; state changes write the hash.
- **Pane sync**: opening a session from Journal or a search hit selects the
  matching project (exactly-one match only) and loads its sessions without
  clearing the open session.
- **Message gutter**: role + humanized timestamp at turn boundaries in the
  archived messages pane.
- **Stable pills**: quick-nav dates accumulate across fetches (newest-first,
  capped) instead of collapsing to the filtered day.
- **Provider badges** on Browse project rows (`getProviderLabel`).
- **Theme toggle** (light → dark → system) in the webapp header, persisted via
  the existing storage adapter.
- **Archived-context rendering**: `ArchiveRenderContext` suppresses the
  "Pending" status/placeholder on tool cards whose results are sibling rows.
- **Project filter labels**: basename (parent appended only on collision),
  sorted; full path in the option `title`.
- **Empty-date action**: "Show latest entries" clears the date filter.
- New i18n keys ×5 locales + type regen.

## Capabilities

### Modified Capabilities

- `archive-journal-ui`: URL state, pane sync, message gutter, stable pills,
  provider badges, archived tool-state, filter labels, empty-state action.
- `static-archive-webapp`: theme toggle joins the reader controls.

## Impact

- **Code**: `src/components/ArchiveBrowser/*` (+ new `archiveRoute.ts`,
  `ThemeToggle.tsx`), `src/contexts/ArchiveRenderContext.ts` (new),
  `unifiedCards/{StatusBadge,ResultBlock}.tsx`, i18n ×5.
- **No hub/Rust change.** Hub follow-ups (journal FTS prefix match,
  message-offset lookup) stay on #18.
- **Tests**: `archiveRoute` unit tests; ConnectGate suite extended; loop evals
  untouched by routing (flag off outside the webapp).
- **Version**: minor → `cchv-v0.9.0`.
