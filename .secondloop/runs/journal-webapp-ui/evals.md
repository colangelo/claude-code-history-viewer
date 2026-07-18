# Acceptance rubric — journal-webapp-ui

Feature: **Archive journal view + unified type scale** (webapp UI for hub journal
entries). Issue #16. Change contract:
`openspec/changes/journal-webapp-ui/` (proposal, `archive-journal-ui` spec,
design). **Frontend only** — no Rust / no hub changes (the API shipped in
v0.6.0: `GET /v1/journal/entries` and the `journal` block of `GET /v1/search`).

## Tier classification

Every acceptance criterion is **frontend-observable → T1 (vitest)**. There are
**no T2 (Rust) criteria**: the hub endpoints already shipped and are exercised by
prior runs' evals (`journal-entries_eval.rs`, `ingest-freshness_eval.rs`); this
run adds no backend surface, so there is **no `journal-webapp-ui_eval.rs`**.

All T1 evals live in the single file
`crates/loop-evals/tests/journal-webapp-ui.eval.test.tsx` (one `it()` per
criterion). They drive the real `ArchiveBrowser` against a **stubbed
`global.fetch`** (a URL router returning hub-shaped fixtures) plus isolated
renders of `ToolUseCard` / `MessageContentDisplay` for the type scale. Nothing
imports a not-yet-existing symbol, so the file loads on the unmodified app and
every criterion fails at runtime (element/query absent), never at import.

| AC  | Tier | Where verified |
|-----|------|----------------|
| AC1 | T1   | `AC1 — Journal/Browse tabs, journal default` |
| AC2 | T1   | `AC2 — day grouping, newest-first, relative label` |
| AC3 | T1   | `AC3 — date picker and project filter` |
| AC4 | T1   | `AC4 — entry card at rest / expanded, lazy session labels` |
| AC5 | T1   | `AC5 — session link drills into Browse` |
| AC6 | T1   | `AC6 — journal hits in search results` |
| AC7 | T1   | `AC7 — empty and error states` |
| AC8 | T1   | `AC8 — unified archive type scale` |
| AC9 | T1   | `AC9 — widened Browse panes and humanized list rows` |
| AC10| T1   | `AC10 — journal strings localized in all five locales` |

## Test-id / DOM contract (what the implementation must expose)

The run spec already mandates the two tab test-ids (the frozen
`archive-viewer-ui` eval depends on them). The journal evals additionally rely on
this small, stable contract — the implementer builds to these:

- `archive-tab-journal` / `archive-tab-browse` — the two tab **`<button>`s**
  (mandated; keyboard-accessible). Activating Browse shows the existing three
  panes unchanged.
- `journal-day-header` — one element per day group (grouping + ordering).
- `journal-entry-card` — one per rendered entry.
- `journal-entry-toggle` — the expand affordance inside each card.
- `journal-session-link` — one per session id inside an **expanded** card.
- `journal-date-picker` — an `<input type="date">` that jumps the feed
  (sets `from`/`to` to the picked day, resets pagination).
- `journal-project-filter` — a `<select>` whose option values carry the
  project path; narrows the feed via the `project` param.
- `journal-empty` — the "no entries in range / older history may not be
  distilled yet" notice (localized).
- `journal-error` — the failed-fetch error state (must not break the tabs).
- `journal-search-section` — the journal-hits section in search results,
  rendered **above** message hits; contains one
- `journal-search-hit` — clickable journal hit that opens the Journal view
  anchored at its `entry_date`.

The existing `archive-search-input` (global search) and `archive-load-more`
test-ids stay. Message-hit search behavior is unchanged.

## Criteria

### AC1 (T1) — Journal-default tabs; Browse unchanged
`ArchiveBrowser` renders the Journal view **by default** with a `Journal |
Browse` switcher (two real buttons); the global search bar stays visible in both.
Activating Browse shows the existing three panes (projects / sessions / messages)
with their current behavior and copy, loading projects through the unchanged
path. **PASS** = journal entry visible on mount with no click; both tab buttons
present; Browse click reveals the three-pane UI and its fetched projects.

### AC2 (T1) — Day-timeline grouping, newest-first, relative recent-day label
Entries spanning multiple `entry_date`s render grouped under day headers,
**newest day first**; entries sharing a date share one header (one card per
project that day). The most-recent **closed** day carries a **humanized relative
label** derived from the `entry_date` string (not the raw ISO), distinct from the
absolute weekday+date label used for older days. **PASS** = header count ==
distinct dates; recent-day header precedes older; same-day cards precede older
card; recent header does not contain the raw ISO and differs from the older
header. *(Visual intent: the most-recent closed day reads "Yesterday". The
executable check asserts humanized-and-distinct rather than the literal word to
stay robust to i18n key naming / `Intl.RelativeTimeFormat` — reviewers confirm
the "Yesterday"-style label by eye.)*

### AC3 (T1) — Date picker jump + project filter
Picking a date refetches `/v1/journal/entries` with matching `from`/`to` (same
day) and renders that day (or the empty notice), resetting pagination. Selecting
a project narrows the feed to that project (refetch carries the `project` param;
other projects' entries disappear). **PASS** = the observed fetch params + the
narrowed render.

### AC4 (T1) — Rich cards, drill-down, lazy session resolution
At rest a card shows project, session count, model, headline, summary, and topic
chips — with **no** `/v1/sessions` request during feed rendering. Expanding
reveals the full summary, open questions, and **one `journal-session-link` per
session id**, resolving session labels via **exactly one** `/v1/sessions?project=`
call, **cached** per project (expanding a second same-project card issues no new
call). **PASS** = at-rest meta present, no session links / open questions at
rest, `listSessions` count 0 before expand → 1 after first expand → still 1 after
a second same-project expand.

### AC5 (T1) — Session link → Browse
Activating a session link switches to the Browse view and loads that session's
messages via the existing path (`GET /v1/sessions/{id}/messages`). **PASS** = the
message fetch for that id fires and the Browse three-pane UI is shown.

### AC6 (T1) — Journal hits in search
A `/v1/search` response carrying a `journal` block renders journal hits
(headline, date, project) in a distinct section **above** message hits;
activating a journal hit opens the Journal view anchored at that entry's date
(`from==to==entry_date`). A response **without** the block renders message hits
exactly as today and shows no journal section. **PASS** = section present and
ordered above messages; anchored refetch + render on activate; graceful absence.

### AC7 (T1) — Empty & error states
An empty range renders the not-yet-distilled notice (a localized string, i.e. an
`settings.archiveHub.journal.*` key — not a raw literal) with no entry cards. A
failing journal fetch renders an error state and leaves the tab switcher working
(Browse still reachable). **PASS** = `journal-empty` / `journal-error` states +
working tabs.

### AC8 (T1) — Unified archive type scale
Conversation message text renders at the **14px** scale; tool-card **headers**
carry an **explicit 13px** size (no size-class-less header inheriting 16px); tool
**ids** render at **11px monospace** — content outranks chrome (14 > 13 > 11).
Verified by computing each element's *effective* font size from its own/ancestor
size class, recognizing both the `text-pxNN` scale **and** arbitrary
`text-[13px]`/`text-[0.8125rem]` values. *(Note: `cn()`/tailwind-merge strips a
bare `text-pxNN` when a `text-color` class follows — the actual root cause of the
current 16px tool headers — so a working fix must ensure the size survives, e.g.
via an arbitrary value or a token/merge change; the eval accepts any mechanism
that yields the right effective size on the rendered card.)* **PASS** = header
== 13, id == 11 (+ `font-mono`), message text == 14, strict 14 > 13 > 11.

### AC9 (T1) — Widened Browse panes + humanized list rows
The Browse **projects pane is 240px (`w-60`)** and the **sessions pane is 320px
(`w-80`)**; session rows render **humanized** timestamps (the raw ISO string is
gone) and **locale-formatted** counts (default `Intl.NumberFormat` grouping, not
the raw integer). **PASS** = the two width classes on the panes; formatted count
present and raw absent; raw ISO timestamp absent from the row.

### AC10 (T1) — Localized journal strings
Every new journal string resolves through `settings.archiveHub.journal.*` keys
present in **all five** locales (en, ko, ja, zh-CN, zh-TW) with identical key
sets (no drift), and a new component actually routes a user-facing string through
those keys (the empty notice is a key, not a raw literal). **PASS** = ≥5 journal
keys in en, identical journal-key sets across all five locales, and the empty
notice resolves through a journal key. (`pnpm run i18n:validate` and
`generate:i18n-types` are part of the gate and must stay green.)

## Notes for the implementer

- Follow the component's existing stale-response protection (generation
  counters) for the new journal/search/session fetches; filter and date changes
  reset pagination.
- **Do not modify** `crates/loop-evals/tests/archive-viewer-ui.eval.test.tsx`
  (frozen; verified still green here) or any other prior frozen eval, and keep
  `hubApi.search()` backward-compatible with it (it mocks `search` as returning a
  plain `HubSearchHit[]`, and asserts `search()` returns the bare array for a
  bare-array response — so surface the `journal` block without breaking that
  shape).
- The AC6 / AC5 evals address the hub purely by URL, so they pass regardless of
  the exact new `hubApi` method names, as long as journal browse hits
  `/v1/journal/entries` and search reads the response's `journal` array.
