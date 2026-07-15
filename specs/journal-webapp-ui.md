# Archive journal view + unified type scale (webapp UI for hub journal entries)

## Description

Client-side feature on the archive browser (issue #16). Full contract:
`openspec/changes/journal-webapp-ui/` (proposal, `archive-journal-ui` spec,
design) — committed on main in this worktree. **Frontend only** (T1 vitest);
no Rust, no hub changes (the API shipped in v0.6.0: `GET /v1/journal/entries`
browse endpoint and the `journal` block in `GET /v1/search`).

What to build (design decisions are final — locked with the user over mockups
2026-07-15):

- **Tabs**: `ArchiveBrowser` gets a `Journal | Browse` switcher; **Journal is
  the default landing view**; Browse is the existing 3-pane UI unchanged; the
  search form stays global above both. Accessible buttons with a selected
  state.
- **Journal view** (`src/components/ArchiveBrowser/JournalView.tsx`):
  reverse-chronological feed from `hubApi.journalEntries()` (new; typed on the
  v0.6.0 response: `entry_date`, `project_path`, `status`, `headline`,
  `summary`, `topics[]`, `open_questions[]`, `session_ids[]`, `model`,
  `generated_at`), grouped client-side by `entry_date` under humanized day
  headers (relative label for the most recent closed day, weekday+date
  otherwise — computed from the `entry_date` string, NOT wall-clock timestamp
  math). Quick-nav pills from the loaded dates, a date picker jumping via
  `from`/`to` params, a project filter via `project`, load-more pagination
  (`limit`/`offset`), and loading / error / empty states — the empty state
  must say older history may not be distilled yet.
- **Entry card** (`JournalEntryCard.tsx`): at rest → project name, session
  count, model, headline, summary clamped to ~2 lines, topic chips. Expanded →
  full summary, open questions (when any), one link per session id. Session
  labels (summary + message count) resolve **lazily on first expand** via one
  `hubApi.listSessions({project})` call per project, cached — never during
  feed rendering. Activating a session link switches to Browse with that
  session's messages loading (reuse the existing `openSessionRef` path).
- **Search**: `hubApi.search()` additionally returns the response's `journal`
  array (tolerate absence → empty). Journal hits render as a distinct section
  ABOVE message hits (headline, date, project); activating one opens the
  Journal view anchored at that entry's date. Message-hit behavior unchanged.
- **Type scale** (shared tokens — desktop/WebUI inherit): conversation
  message text at the 14px scale with relaxed leading (replace the `prose-xs`
  usage for message content); tool-card headers get an EXPLICIT 13px
  medium-weight token (today they carry no size class and inherit 16px); tool
  ids 11px mono; update `src/components/renderers/styles.ts` layout tokens
  coherently. Browse panes: projects `w-48`→`w-60` (240px), sessions
  `w-64`→`w-80` (320px); list rows at 13px with 11px meta; session-list
  timestamps humanized via a small `Intl`-based util (no date library) and
  counts locale-formatted.
- **i18n**: every new user-visible string through
  `settings.archiveHub.journal.*` keys added to ALL FIVE locales (en, ko, ja,
  zh-CN, zh-TW) + `pnpm run generate:i18n-types`; `pnpm run i18n:validate`
  must pass.

Constraints:

- **Prior-run regression tests will need setup updates, not weakening:**
  `crates/loop-evals/tests/archive-viewer-ui.eval.test.tsx` and
  `src/test/ArchiveBrowser.test.tsx` render `ArchiveBrowser` and assert
  Browse-pane content directly; with Journal as the default landing view they
  must be updated to activate the Browse tab in setup first. Keep every
  existing assertion intact — only the navigation setup may change.
- Evals for THIS run: import only surfaces that exist today (`ArchiveBrowser`
  root via `@/`, `hubApi` module for `vi.spyOn`/mocks); never fetch a live
  server; missing-behavior must fail at runtime (element/query absent), not
  at import time.
- Follow the component's existing stale-response protection (generation
  counters) for the new fetches; filter/date changes reset pagination.
- Lint-clean, type-correct, existing tests keep passing.

## Acceptance Criteria

- (T1) With a valid config and mocked hubApi, `ArchiveBrowser` renders the Journal view by default with a `Journal | Browse` tab switcher, and activating Browse shows the existing three panes with their current behavior.
- (T1) Mocked entries spanning multiple `entry_date`s render grouped under day headers newest-day-first, with the most recent closed day carrying a relative "yesterday"-style label derived from the entry_date string.
- (T1) Picking a date refetches entries with matching `from`/`to` params and renders that day (or the empty notice), and selecting a project filter narrows the rendered entries to that project.
- (T1) An entry card shows project, session count, headline, clamped summary, and topic chips at rest; expanding reveals the full summary, the open questions, and one link per session id — and the sessions-list request happens only on first expand, never at feed render.
- (T1) Activating a session link switches to the Browse view and requests that session's messages via the existing message-fetch path.
- (T1) A search response containing a `journal` block renders journal hits in a distinct section above message hits, activating a journal hit shows the Journal view anchored at that entry's date, and a response without the block renders message hits exactly as today.
- (T1) An empty journal range renders the not-yet-distilled notice, and a failing journal fetch renders an error state without breaking the tab switcher.
- (T1) Message-text containers carry the 14px-scale content classes while tool-card headers carry an explicit 13px-scale token and tool ids an 11px mono token — no size-class-less tool header remains in the rendered card.
- (T1) The Browse projects and sessions panes carry the widened width classes (240px/320px scale) and session rows render humanized timestamps and locale-formatted counts.
- (T1) Every new journal UI string resolves through i18n keys present in all five locale files, with no raw user-facing string literals in the new components.
