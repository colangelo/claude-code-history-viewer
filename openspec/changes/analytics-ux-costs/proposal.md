# Analytics UX + cost primacy

## Why

A live walkthrough of the archive webapp (2026-07-26, v0.16.0 on m4m) surfaced
five UX defects in and around the Analytics tab. The sharpest: selecting a
project in the scope dropdown makes **cost disappear entirely** — and cost is
the first metric a user looks for. Root cause verified against the live hub:
`/v1/stats/projects/{key}` returns no `model_distribution`, so the client-side
pricing that produces the global "Estimated Cost" figure has nothing to price,
and `ProjectStatsView` renders no cost field at all. Alongside that: the global
search bar renders over Analytics where it doesn't belong, activating a search
hit leaves the (unbounded-height) results overlay covering the destination
view, the 30/90/all range control is too coarse, and the activity heatmap —
one of the most-read charts — sits in the last row while tool counts occupy
prime slots.

## What Changes

- **Search containment** (Browse/Journal search overlay, `ArchiveBrowser`):
  - Journal search results get the same height cap message results already
    have; no more 3000-px result walls.
  - Activating any search hit (journal or message) dismisses the results
    overlay — the user asked to go somewhere, so land them there. Query text
    stays in the input.
  - The search bar and results do not render in the Analytics view. `/`
    pressed in Analytics switches to Journal and focuses search (search lives
    with content). Result state survives the round-trip.
- **Project-scope cost** (hub + webapp):
  - Hub `/v1/stats/projects/{identity_key}` gains `model_distribution`, same
    shape and semantics as the global endpoint's.
  - `ProjectStatsSummary` (TS) gains `model_distribution`; `ProjectStatsView`
    prices it with the existing estimator and renders the same cost surfaces
    the global view has: estimated-cost metric, Estimated/coverage badges, and
    a per-model distribution section with $ figures.
- **Cost primacy**: Estimated Cost becomes the **first** metric card at both
  scopes (with pricing-coverage as its sub-line); the "N tools used" metric
  card is dropped (its information lives in the Most Used Tools chart).
- **Range granularity**: the 30/90/all buttons become presets
  (7/14/30/60/90/180/365/all) plus a custom "last N days" numeric input.
- **Layout reorder**: the activity heatmap moves to the first section row at
  both scopes; Most Used Tools moves down.
- **Cost-centrality audit** (documented, partially deferred): top-projects,
  provider distribution, and daily trend rows expose only token totals — no
  per-model split exists at those grains, so client-side pricing there would
  be invented. Recorded as a follow-up hub capability rather than faked now.

## Capabilities

### New Capabilities

_None — all changes land in existing capability areas._

### Modified Capabilities

- `archive-analytics`: project-scope stats response includes
  `model_distribution`; cost is a first-class headline metric at both scopes;
  range control offers preset + custom windows; heatmap precedes tool charts.
- `static-archive-webapp`: the global search overlay is scoped to the content
  views (Journal, Browse); hit activation dismisses results; journal results
  are height-capped.

## Impact

- `crates/hub/src/stats.rs` + `stats_api.rs` — per-model rollup query at
  project scope, response struct field, tests.
- `src/services/hubApi.ts`, `src/types/stats.types.ts` — response typing.
- `src/components/ArchiveBrowser/index.tsx` — search overlay containment,
  `/` behavior, AnalyticsView slot.
- `src/components/ArchiveBrowser/AnalyticsView.tsx` — range control.
- `src/components/AnalyticsDashboard/views/{GlobalStatsView,ProjectStatsView}.tsx`
  — metric card order, cost surfaces, section order.
- i18n: new keys ×5 locales (range presets, custom-days control, cost card).
- No breaking API change: `model_distribution` is additive; old webapp ↔ new
  hub and new webapp ↔ old hub both degrade gracefully (absent field → no cost
  shown, as today).
