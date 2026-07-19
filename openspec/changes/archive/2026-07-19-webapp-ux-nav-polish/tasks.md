# Tasks: webapp-ux-nav-polish

Acceptance authority: `specs/*/spec.md` deltas. All frontend; no Rust.

## 1. Routing

- [x] 1.1 `archiveRoute.ts`: parse/format for `#/journal[/date]`,
  `#/browse[/session/<ref>]`, `#/search/<q>` + unit tests.
- [x] 1.2 ArchiveBrowser wiring: route-aware state initializers, mount
  deep-link fetches, state→hash writes with self-write guard, hashchange
  listener (back/forward), search hash on submit.
- [x] 1.3 `enableHashRoutes` prop, on only in the webapp ConnectGate —
  embedded desktop/WebUI never touch the URL.

## 2. Browse coherence

- [x] 2.1 Pane sync: `SessionOpenContext` from Journal cards and search hits;
  exactly-one project match selects it and loads sessions without clearing
  the open session.
- [x] 2.2 Role/timestamp gutter at turn boundaries in the messages pane.
- [x] 2.3 Provider badge on project rows.
- [x] 2.4 `ArchiveRenderContext`: suppress "Pending" status/placeholder on
  archived tool cards (StatusBadge, ResultBlock).

## 3. Journal polish

- [x] 3.1 Quick-nav pills from an accumulated date union (newest-first,
  capped 21) that survives date filters.
- [x] 3.2 Filter labels: basename, parent on collision, sorted, full path in
  `title`; date-mirror callback for hash routing.
- [x] 3.3 Empty-date action button clearing the filter.

## 4. Chrome

- [x] 4.1 ThemeToggle (light→dark→system) in the header, persisted through
  the storage adapter; localized labels.

## 5. Gate

- [x] 5.1 i18n keys ×5 locales, `generate:i18n-types`, `i18n:validate`.
- [x] 5.2 `pnpm tsc --build .`, `vitest run`, `pnpm lint` green.
- [x] 5.3 Rebuilt `dist-archive` verified in a browser (Playwright): deep
  links, back button, theme toggle, gutter, pane sync, mobile.
