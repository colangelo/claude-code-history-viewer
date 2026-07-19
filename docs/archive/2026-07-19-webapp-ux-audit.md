# Archive webapp UX audit — 2026-07-19

Method: Playwright audit of the **live** hub webapp (`https://m4m.cat-bluegill.ts.net:8788/`,
cchv-v0.7.0, real data) at 1440×900 light/dark and 390×844 mobile, plus computed-style
probes and a source review of `src/components/ArchiveBrowser/*`. All font sizes below are
measured `getComputedStyle` values, not guesses.

## Headline problems

### 1. Type is too small everywhere (confirmed)

The base font is 16px, but **nothing in the app renders at ≥15px**. Measured:

| Element | Size | Note |
|---|---|---|
| App title (`h1`) | 14px | |
| Journal card headline | 14px | the *largest* text in the app |
| Day group header | 13px | **smaller than the headlines it groups** |
| Card summary (primary reading text) | 13px | |
| Tabs, search input, list rows | 13px | |
| Card meta, topic chips, date pills | 11px | pervasive |

Leaf-element font-size histogram of a rendered session (messages pane):
`11px×7, 12px×8, 13px×4, 14px×1` — chrome outweighs content. The v0.7.0 scale was tuned
for a dense desktop tool; in a browser it reads as too small, and the webapp exposes **no
font-size control**: `--app-font-scale` exists in CSS but is only set by the desktop/WebUI
bootstrap (`useAppInitialization`), never by `archive-main.tsx`.

Also: cards and messages stretch the full viewport width (~1410px text measure at 1440px —
roughly 200+ characters per line; comfortable reading measure is ~70–90ch). No content
column max-width anywhere.

### 2. Browse is unusable on mobile (critical)

The three Browse panes are fixed-width `shrink-0` (240px + 320px + flex) in an
`overflow-hidden` row. At 390px viewport the sessions pane is clipped and the **messages
pane is entirely off-screen and unreachable** (no horizontal scroll possible). Opening a
session from a Journal card on a phone lands on invisible content. Journal itself works on
mobile; Browse does not.

### 3. No URL state at all

Refresh always resets to the Journal landing view. No deep links to a session, a date, or
a search — nothing is shareable/bookmarkable and the back button does nothing. For a
web-hosted archive this is a core missing capability.

## Per-view findings

### Journal

- Meta row leads with the full absolute project path in 11px mono
  (`/Users/ac/_sync/dev/sergente`); project identity should be the basename with the path
  demoted (tooltip).
- Quick-nav pills are derived from currently-loaded groups, so **they vanish when a date
  filter is set** — after any jump the only way out is "Clear date"; you can't hop between
  days. Pills are also 11px with ~20px hit areas (touch guideline is 44px).
- Project filter is a native `<select>` of 31 unsorted absolute paths (incl.
  second-loop worktree noise).
- Expand affordance is only the headline row; clicking the summary/card body does nothing.
- Session links inside an expanded card are often identical boilerplate ("You are handling
  an agent-relay message… ×4) with no timestamps/duration to tell them apart; unresolved
  labels render as raw numeric ids.
- Empty date state is fine textually but offers no "jump to nearest day with entries".
- Dark mode: native date input ignores the theme (`color-scheme`), calendar icon nearly
  invisible.

### Browse

- No provider badge: `home-network · m4m.local` appears twice (claude vs codex ingest)
  with **no way to distinguish the rows**, though the API returns `provider`.
- Messages have **no per-message role/timestamp** — user bubbles are colored, but
  assistant/tool blocks carry no attribution and *nothing* shows when anything happened.
- Session opened from Journal/search leaves the Projects/Sessions panes in "Select a
  project" state — disorienting mismatch.
- `totalCount` is fetched but never shown ("Load more" with no "showing 200 of 1,234").
- Search-hit → session lands at page 1 with no scroll-to/highlight of the matched message;
  the user must re-find the hit manually.
- Tool-use cards show live-state "Pending" chips on archived executions (misleading; the
  results sit right below).
- 200-message pages of heavy renderers, no virtualization (the desktop viewer virtualizes).

### Search

- FTS highlight markup renders literally: snippets display `<b>distill</b>` as text. Bug.
- Results panel: no hit count, no dismiss/clear affordance, `max-h-40` (~3.5 rows) for up
  to 50 hits, and it pushes the tab bar down (layout shift).
- Journal FTS is whole-token: "distill" → 0 journal hits while "Hermes" → 4, because
  entries say "distiller"/"distillation". Feels broken; needs prefix matching hub-side or
  at least messaging.
- No `/` or ⌘K shortcut; no loading state on the button; hits lack timestamps.

### Chrome / global

- No indication of which hub you're connected to, or that same-origin Tailscale identity
  auth is in effect; "Disconnect" on an auto-connected session just bounces back on reload.
- Manual connect **requires** a token, so a tokenless-but-identity-authed hub can't be
  added from a non-hub origin (e.g. locally-served build against the tailnet hub).
- No theme toggle, no font-size control, no language switcher.
- Tabs are `role=tablist` without arrow-key navigation.

## What works well

Same-origin auto-connect is seamless; stale-response generations everywhere; lazy session
resolution on card expand; day grouping with humanized headers; message renderers
(tool cards, diffs, markdown) are rich; dark theme palette (minus the date input); no
console errors; Journal on mobile is serviceable.

## Enhancement plan (phased)

**Phase 1 — Readability (the complaint):**
new `text-px15/16` utilities; headline 14→16 semibold, day header 13→15 semibold (above
headlines at last), summary/list rows 13→14, conversation content ≥15, meta/chips/pills
11→12, tabs/inputs 13→14, title 14→16; reading-measure column (`max-w-4xl`) for journal
feed and messages pane; **font-size control** (A−/A/A+, persisted, drives
`--app-font-scale`, webapp default 1.1); dark date-input `color-scheme` fix.

**Phase 2 — Mobile Browse:** stacked drill-down (<md): one level visible at a time
(projects → sessions → messages) with back buttons; bigger touch targets.

**Phase 3 — Search UX:** parse `<b>` markers into real highlights; hit count + clear (×);
loading state; `/` focus shortcut; timestamps on hits.

**Phase 4 — Navigation & state:** hash routing/deep links (`#/journal/2026-07-17`,
`#/session/<id>`, `#/search/<q>`); sync Browse pane selection when arriving from
Journal/search; "showing X of Y" + role/timestamp gutter per message; stable quick-nav
pills that survive date filters.

**Phase 5 — Chrome polish:** hub identity in header; provider badges in Browse; theme
toggle; suppress "Pending" chips in archive context; project filter with short names;
empty states with actions. Hub-side follow-ups: journal FTS prefix matching; a
message-offset lookup so search hits can land on the matched page.

Phases 1–3 are client-only and low-risk; phase 4 is client-only but touches state
management; phase 5 includes two hub API follow-ups.
