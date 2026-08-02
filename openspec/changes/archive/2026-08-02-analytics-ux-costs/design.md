# Design — analytics-ux-costs

## Context

Analytics (webapp `AnalyticsView` → reused desktop `GlobalStatsView` /
`ProjectStatsView`) landed in the web-only pivot (cchv-v0.14.0). Cost is
computed **client-side**: `calculateGlobalCostSummary` prices
`model_distribution` (per-model token counts) against the TS pricing table.
The hub's server-side `total_cost_usd` (sum of message-reported `costUSD`) is
effectively empty on this archive (`cost_reported_messages: 0` live) and is
not the figure the UI shows. The project stats response carries no
`model_distribution`, so project scope has no cost path at all — verified live
2026-07-26.

The search bar + results overlay live in `ArchiveBrowser/index.tsx` above the
tablist, rendered in every view. Message hits are capped `max-h-72`; journal
hits are uncapped. `handleActivateJournalHit`/`handleActivateHit` navigate but
do not dismiss results.

## Goals / Non-Goals

**Goals:**

- Cost visible and first at both scopes, using the one pricing path that
  exists (client-side estimator over `model_distribution`).
- Search overlay contained: capped, dismissed on activation, absent from
  Analytics.
- Finer range control without a redesign of the toolbar.
- Heatmap prominent; tool-count noise demoted.

**Non-Goals:**

- Server-side pricing (hub knows no prices; pricing stays client-side).
- Costing grains with no per-model split (top projects, provider
  distribution, daily trend) — recorded as a possible future hub aggregation,
  not faked now.
- Desktop (Tauri) analytics behavior: `showBillingBreakdown` and the
  conversation-split path stay untouched; the desktop is retired but its
  code paths are not being reworked here.
- The `index.tsx` monolith split (separately planned).

## Decisions

- **Hub, not frontend, closes the project-cost gap.** `stats.rs` grows a
  per-model rollup for the identity-scoped message set — same
  `GROUP BY model_name` shape as the global query, over the already-
  materialized deduped scope, so the cost profile measured in #24 gains one
  cheap statement, not a second scope materialization. Additive field; serde
  default keeps old-hub/new-webapp and new-hub/old-webapp both working.
- **Estimated Cost becomes metric card 1** (value `$…`, sub-line pricing
  coverage), tokens card 2, messages 3, session time 4. The "N tools used"
  card is dropped — the count of distinct tools is the least informative
  number on the page and its content lives in the Most Used Tools chart.
  Applied to both views for symmetry.
- **Range control = preset `<select>` + custom days input.** Presets
  7/14/30/60/90/180/365/all replace the three buttons; choosing "Custom…"
  reveals a numeric input (positive integer, applied on commit). One control,
  keyboard-accessible, no date-picker dependency. The existing `from=`
  ISO-date request param already expresses every window, so the hub is
  untouched by this item.
- **Search scoping over search redesign.** The bar renders only for
  Journal/Browse; Analytics gets the toolbar it already owns. `/` in
  Analytics switches to Journal then focuses — "/" expresses intent to
  search, so it lands where search lives. Dismissal on hit-activation reuses
  `handleClearSearch` (which already preserves the query). The journal
  section gets the message section's `max-h-72 overflow-y-auto`. This keeps
  the current interaction model (results as a transient overlay above the
  feed) while fixing what made it feel broken; a dedicated search *view* is a
  bigger rethink deliberately not attempted here.
- **Layout order (both scopes):** row 1 = Activity Heatmap | Model
  Distribution (cost lives there — primacy); then provider/top-projects
  (global), then tools/skills/subagents, then the remaining sections
  unchanged.

## Risks / Trade-offs

- **Old hub + new webapp**: project scope has no `model_distribution` → cost
  card must render an explicit "n/a at this hub version" state, not `$0.00`.
  (A `$0` would be read as "this project cost nothing".)
- **Pricing coverage at project scope** can be 0% (e.g. all-unknown models):
  same treatment — show coverage, never bare zero-dollar confidence.
- **Tabs shift up ~44 px entering Analytics** (search bar unrendered). Judged
  acceptable: per-view toolbars are a normal pattern and the alternative
  (search visibly present but inert/incongruous) is the defect being fixed.
- **Rust validation on this machine**: cargo may not run locally (per
  release-gate-runner notes); hub tests then ride CI. Mitigation: keep the
  SQL change mirrored on the global query's tested shape and verify the live
  response shape after deploy.
