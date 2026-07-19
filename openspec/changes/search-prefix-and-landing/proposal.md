# Proposal: search-prefix-and-landing

## Why

The v0.10.1 batch (issues #19/#20/#17/#21, split from the closed UX epic #18):
both search surfaces parse `q` with `websearch_to_tsquery('simple', …)` — no
stemming, no prefixing — so `distill` finds nothing distilled as "distiller"
(#19); activating a message hit opens its session at page 1 with the match
possibly hundreds of messages away (#20); a transient pg1 DNS flake 500s every
read because the pool holds no warm connections (#17); and four small client
gaps remain from the UX audit (#21).

## What Changes

- **Hub — FTS prefix matching (#19):** `fts::prefix_tsquery` builds a
  `'tok':* & …` variant for PLAIN queries; both search queries OR it into the
  websearch parse via a CTE. Advanced syntax (phrases/OR/negation) gets no
  variant — exact semantics preserved. Unit + integration tested.
- **Hub — hit position (#20):** `/v1/search` results gain `position` — the
  0-based index of the hit in its session's browse ordering
  (`timestamp ASC NULLS LAST, seq, id`), NULLS-LAST-exact via a counting
  subquery.
- **Hub — pool resilience (#17):** `min_connections(2)` +
  `acquire_timeout(5s)` so warm connections ride out DNS flakes (mitigation:
  established conns need no re-resolution; `test_before_acquire` pings
  without DNS).
- **Client — hit landing (#20):** a hit with `position` opens the page
  CONTAINING the match (windowed messages: `windowStart`), scrolls the
  matched message to center and highlights it; "Load earlier" extends the
  window upward; progress shows a range (`401–450 of 450`) for non-zero
  windows. Older hubs (no `position`) → page 1 exactly as before.
- **Client — riders (#21):** humanized timestamps on message hits; `/`
  focuses the search input; tablist ArrowLeft/Right switch tabs; manual
  connect works tokenless against identity-authed hubs (empty token valid in
  form, probe, and stored config).
- i18n: 2 new keys + token-label wording ×5 locales.

## Capabilities

### Modified Capabilities

- `archive-search-api`: prefix matching requirement; `position` on hits.
- `archive-journal-ui`: search-hit landing behavior; rider affordances.
- `static-archive-webapp`: tokenless manual connect.

## Impact

- **Code**: `crates/hub/src/{fts.rs (new), search.rs, journal.rs, lib.rs}`,
  `.sqlx` regenerated; `src/services/hubApi.ts`,
  `src/components/ArchiveBrowser/{index.tsx, ConnectGate.tsx,
  hubConfigStorage.ts}`, i18n ×5.
- **No migration.** Hub deploy = binary swap only (§2b); daemons unaffected.
- **Tests**: 6 fts unit + 3 hub integration tests; ConnectGate tokenless
  test; 920 frontend tests green; e2e browser-verified against a local
  v0.10.1 hub with a seeded 450-message session.
- **Version**: `cchv-v0.10.1` (user-directed patch line).
