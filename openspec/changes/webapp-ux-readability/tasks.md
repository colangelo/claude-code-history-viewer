# Tasks: webapp-ux-readability

Acceptance authority: `specs/*/spec.md` deltas. All frontend; no Rust.

## 1. Type scale & measure

- [x] 1.1 `index.css`: add `.text-px15` / `.text-px16` utilities (same
  `--app-font-scale`-reactive pattern as px6–px14).
- [x] 1.2 `renderers/styles.ts`: messageText px14→px15, titleText px13→px14,
  bodyText/monoText px12→px13, toolId px11→px12; message prose kept at the same
  computed size as messageText.
- [x] 1.3 Archive components: headline px16, day header px15 semibold, summary/rows
  px14, meta/chips/pills px12, tabs/inputs px14 + h-9, pane titles px12; journal feed
  and messages content in a `max-w-4xl` column.
- [x] 1.4 Dark-mode date input `color-scheme` fix.

## 2. Font-size control

- [x] 2.1 `FontScaleControl.tsx`: A−/A+ buttons stepping 0.8–1.4 by 0.1, applied to
  `--app-font-scale` on `:root`, persisted (`cchv.archiveWeb.fontScale`), webapp
  default 1.1; aria-labels localized.
- [x] 2.2 Mount in the ConnectGate connected header next to the hub host display.

## 3. Mobile Browse

- [x] 3.1 Stacked drill-down below `md`: exactly one pane visible (messages when a
  session is open, else sessions when a project is selected, else projects);
  `md:` restores the 3-pane layout.
- [x] 3.2 Localized back buttons (`md:hidden`) in the sessions and messages panes.
- [x] 3.3 Date pills / small buttons get ≥py-1 touch padding.

## 4. Search UX

- [x] 4.1 `searchSnippet.tsx`: parse `<b>…</b>` runs into `<mark>` nodes (never
  innerHTML; unpaired markers degrade to plain text) + unit tests.
- [x] 4.2 Results header: localized hit count + clear-results (×) button; list
  max-height raised.

## 5. Quick wins

- [x] 5.1 Header shows connected hub host (full URL in tooltip).
- [x] 5.2 Messages pane header shows "N of M messages" from `totalCount`.
- [x] 5.3 Journal card meta leads with project basename (full path in `title`).

## 6. Gate

- [x] 6.1 i18n keys ×5 locales, `generate:i18n-types`, `i18n:validate`.
- [x] 6.2 `pnpm tsc --build .`, `vitest run`, `pnpm lint` green.
- [x] 6.3 Rebuilt `dist-archive` verified in a browser (Playwright) desktop + mobile
  + dark, against the live hub.
