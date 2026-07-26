# Tasks — analytics-ux-costs

## 1. Search containment (webapp)

- [x] 1.1 Cap the journal-hits section (`max-h-72 overflow-y-auto`, matching message hits) in `ArchiveBrowser/index.tsx`
- [x] 1.2 Dismiss the results overlay on hit activation: `handleActivateJournalHit` and `handleActivateHit` call the existing clear path (query text preserved)
- [x] 1.3 Render the search form + results only when `view !== "analytics"`; state survives the round-trip
- [x] 1.4 `/` in Analytics switches to Journal and focuses the search input
- [x] 1.5 Vitest coverage: activation dismisses results; analytics hides the search surface; results survive analytics round-trip

## 2. Hub: project-scope model distribution

- [x] 2.1 `crates/hub/src/stats.rs`: per-model rollup over the identity-scoped deduped set (mirror of the global `model_distribution` statement)
- [x] 2.2 `stats_api.rs` project response gains `model_distribution` (additive, serde-default)
- [x] 2.3 Hub tests: known identity returns per-model rows scoped to that identity and window; empty-model messages fold as the global query does
- [x] 2.4 Validate: ran locally (13/13, throwaway local pg db) + clippy/fmt clean; live-shape check still owed post-deploy

## 3. Webapp: project cost surfaces

- [ ] 3.1 `ProjectStatsSummary` TS type gains optional `model_distribution`; `hubApi.statsProject` unchanged otherwise
- [ ] 3.2 `ProjectStatsView`: estimated-cost metric card (coverage sub-line), Estimated/coverage badges, model-distribution section with $ figures — reusing the global view's estimator and components
- [ ] 3.3 Old-hub degradation: absent `model_distribution` → explicit "not available" state on the cost card, never `$0.00`

## 4. Cost primacy + layout (both scopes)

- [ ] 4.1 Metric card order Global: Cost | Tokens | Messages | Session time; "N tools used" card dropped
- [ ] 4.2 Metric card order Project: same
- [ ] 4.3 Section order both views: row 1 = Activity Heatmap | Model Distribution; tools/skills/subagents after
- [ ] 4.4 Audit note: token-only grains (top projects, provider distribution, daily trend) documented as unpriceable client-side; no invented cost rendered

## 5. Range control

- [ ] 5.1 `AnalyticsView`: preset select (7/14/30/60/90/180/365/all) + "Custom…" numeric days input replacing the three buttons
- [ ] 5.2 Custom input validates positive integer, applies on commit, survives scope switches
- [ ] 5.3 Vitest coverage for window → `from=` mapping incl. custom days

## 6. i18n + gate

- [ ] 6.1 New keys ×5 locales (presets, custom days, cost card, n/a state); `generate:i18n-types`; `i18n:validate`
- [ ] 6.2 Full gate: `pnpm tsc --build .`, `pnpm vitest run`, `pnpm lint`

## 7. Eyeball + close-out

- [ ] 7.1 Real-window screenshots (headed Chromium + `screencapture`), both themes: analytics global with cost card first + heatmap row 1; project scope showing cost; range control open; search-from-journal activation landing clean
- [ ] 7.2 Update `openspec/specs/` from the deltas on archive (sync happens at archive time)
