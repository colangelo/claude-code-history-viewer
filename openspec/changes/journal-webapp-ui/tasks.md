# Tasks: journal-webapp-ui

Contract-shaped; `specs/archive-journal-ui/spec.md` is the acceptance
authority. All frontend — T1 vitest tier; no Rust changes.

## 1. Data layer

- [ ] 1.1 `hubApi.ts`: add `journalEntries(config, {project?, from?, to?,
  limit?, offset?})` typed on the v0.6.0 browse response; extend `search()`
  to surface the `journal` block (tolerating its absence); export
  `JournalEntry` / `JournalSearchHit` types.
- [ ] 1.2 Small format util: humanized entry-date labels (Yesterday/weekday
  via `Intl.DateTimeFormat`, relative labels computed from `entry_date`
  strings, not timestamps) + locale-formatted counts.

## 2. Journal view

- [ ] 2.1 Tab switcher in `ArchiveBrowser` (`view: journal|browse`, journal
  default; accessible buttons; search bar global above both).
- [ ] 2.2 `JournalView.tsx`: paged feed grouped by `entry_date` with day
  headers, quick-nav date pills, date picker jump, project filter, load-more,
  loading/error/empty states (empty notes un-distilled history).
- [ ] 2.3 `JournalEntryCard.tsx`: rich-at-rest card (meta, headline, 2-line
  summary clamp, topic chips); expand → full summary + open questions +
  session links; lazy label resolution (one sessions-list fetch per project,
  cached); session link → Browse with session open.
- [ ] 2.4 Search results: journal-hit section above message hits; activating
  a journal hit → Journal view anchored at its date.

## 3. Type scale

- [ ] 3.1 Shared tokens: conversation prose to 14px/relaxed; tool-card header
  to explicit 13px medium; tool ids 11px mono; update
  `renderers/styles.ts` layout constants coherently (desktop/WebUI inherit).
- [ ] 3.2 Browse panes: projects ~240px, sessions ~320px, rows 13px / meta
  11px, humanized dates + formatted counts in lists.

## 4. i18n + ship

- [ ] 4.1 `settings.archiveHub.journal.*` keys in en, ko, ja, zh-CN, zh-TW;
  `generate:i18n-types`; `i18n:validate` green.
- [ ] 4.2 Quality gate (tsc, vitest incl. new T1 tests, lint, i18n, rust gate
  untouched but run).
- [ ] 4.3 Release `cchv-v0.7.0`; `just archive-web-build`; stage bundle +
  relay infra for the hub `static_dir` rsync; e2e on the live hub (journal
  loads, yesterday visible, drill-down works, search shows journal section).
- [ ] 4.4 Close #16; note follow-ups (heatmap strip, dense toggle) stay open
  ideas, not issues, unless requested.
