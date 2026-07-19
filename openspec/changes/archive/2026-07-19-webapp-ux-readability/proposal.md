# Proposal: webapp-ux-readability

## Why

A 2026-07-19 UX audit of the live webapp (`docs/archive/2026-07-19-webapp-ux-audit.md`,
Gitea #18) confirmed the user-reported complaint: nothing renders at ≥15px (headlines
14px, day headers 13px — smaller than the headlines they group — meta 11px), cards
stretch to ~200ch line lengths, and the webapp exposes no font-size control even though
`--app-font-scale` exists. Two further defects: Browse is unusable on mobile (messages
pane off-screen inside an `overflow-hidden` row — session links from Journal land on
invisible content), and search snippets render FTS `<b>` markers as literal text with no
result count or dismiss affordance.

This change implements phases 1–3 of #18 (readability, mobile Browse, search UX).
Phases 4–5 (routing/deep links, chrome polish) and hub-side follow-ups stay on #18.

## What Changes

- **Type scale bump** (shared renderer tokens + archive components): conversation text
  14→15px, tool-card headers 13→14px, tool ids 11→12px, card headlines 14→16px, day
  headers 13→15px semibold, summaries/list rows 13→14px, meta/chips/pills 11→12px,
  tabs/inputs 13→14px (h-8→h-9). New `text-px15`/`text-px16` utilities. The
  desktop/WebUI viewer inherits the token changes.
- **Reading measure**: journal feed and Browse messages constrained to a `max-w-4xl`
  centered column.
- **Font-size control** in the webapp header: A−/A+ stepping `--app-font-scale`
  0.8–1.4, persisted in localStorage, webapp default **1.1**.
- **Mobile Browse**: below `md`, the three panes become a stacked drill-down (one level
  visible: projects → sessions → messages) with back buttons; pill/button touch targets
  enlarged.
- **Search UX**: `<b>` FTS markers parsed into real `<mark>` highlights (no innerHTML);
  hit count + clear-results button; taller results list.
- **Quick wins**: hub host shown in the header; "N of M messages" progress in the
  messages pane; project basename (full path in tooltip) on journal cards; dark-mode
  `color-scheme` fix for the date input.
- New i18n keys ×5 locales + type regen.

## Capabilities

### Modified Capabilities

- `archive-journal-ui`: type scale requirement re-tuned (sizes above, reading measure,
  responsive Browse); search-hit snippets requirement gains highlight rendering.
- `static-archive-webapp`: header gains hub identity + persisted font-size control.

## Impact

- **Code**: `src/index.css` (px15/16 utilities), `src/components/renderers/styles.ts`
  (shared tokens), `src/components/ArchiveBrowser/*` (all four files + new
  `FontScaleControl.tsx`), `src/utils/searchSnippet.tsx` (new), i18n locales ×5.
- **No hub/Rust change.**
- **Tests**: snippet-highlight unit tests; existing vitest suites stay green;
  `i18n:validate`.
- **Version**: minor → next `cchv-v0.8.0` when released.
