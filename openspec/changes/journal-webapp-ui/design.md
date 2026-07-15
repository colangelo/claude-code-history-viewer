# Design: journal-webapp-ui

## Context

The hub's journal surface (v0.6.0: `GET /v1/journal/entries`, `journal` block
in `/v1/search`) has no client. The archive browser
(`src/components/ArchiveBrowser/index.tsx`, 439 lines) is a single component
with three panes and a search bar, using `services/hubApi.ts` (plain fetch +
bearer/identity auth). Message rendering reuses the app's shared renderers,
whose type scale is broken: tool-card headers carry no size class (inherit
16px) while message prose is `prose-xs` (12px).

Design fixed interactively 2026-07-15 with mockups (visual companion):
day timeline · rich cards · journal-default tabs · 14/13/11 type scale with
240/320px panes.

## Goals / Non-Goals

**Goals:** answer "what did I work on yesterday / on date D" in one glance;
make distilled entries the archive's front door; one coherent type hierarchy
where conversation content outranks tool chrome; keep Browse behavior intact.

**Non-Goals:** calendar heatmap (later garnish on the feed), dense-mode
toggle, dropped-threads report (#15), hub/Rust changes, mobile-specific
layouts beyond what exists.

## Decisions

1. **Tabs inside `ArchiveBrowser`, not a new mode at the app level.** The
   component already IS the archive mode in all three hosts (static webapp,
   desktop, WebUI); a local `view: "journal" | "browse"` state gives every
   host the feature with zero host-side wiring. Alternative (new top-level
   mode per host) rejected: three integration points for the same thing.
2. **Client-side day grouping over a server "days" endpoint.** The browse
   endpoint already returns newest-first entries with `entry_date`; grouping
   ~30 rows client-side is trivial and keeps the hub untouched. Quick-nav
   pills derive from the loaded page's distinct dates.
3. **Lazy session-label resolution at expand time.** Cards carry only
   session ids; labels come from one `GET /v1/sessions?project=` per expanded
   card (cached per project). Feed rendering stays one request per page — no
   N+1. Rejected: embedding labels in the hub response (API change for a
   client nicety).
4. **Journal search hits jump to the date, not a dedicated entry page.** The
   feed anchored at `from=to=entry_date` IS the entry's context; a separate
   detail route adds surface without value at this scale.
5. **Type scale lands in shared tokens** (`renderers/styles.ts` layout
   constants + the tool-card header, plus `MessageContentDisplay` prose
   classes): the bug is shared, so the fix is shared; the desktop viewer has
   the same 16px-tool-header problem. Archive-only overrides rejected as
   drift-prone duplication.
6. **Humanized dates via a small `Intl`-based util** (relative labels for
   ≤2 days, `Intl.DateTimeFormat` otherwise; `Intl.NumberFormat` for counts).
   No date library added.

## Risks / Trade-offs

- [Type-scale change ripples into desktop viewer screenshots/tests] →
  intended (same bug there); T1 evals assert relative hierarchy, not exact
  pixels, so they survive minor tuning.
- [Entry dates are logical days (04:00 UTC fold)] → "Yesterday" labels
  computed from `entry_date` directly, not wall-clock arithmetic on
  timestamps; the open day is simply absent, which the empty-state copy
  explains.
- [Search journal block absent on pre-0.6.0 hubs] → treat missing block as
  empty list; the section just doesn't render.
- [Feed pagination across a project filter change] → filter/date changes
  reset pagination state (same generation-counter pattern the component
  already uses for stale-response protection).

## Migration Plan

Pure client feature: ships in the static webapp bundle (`just
archive-web-build` → hub `static_dir` rsync by infra) and in the next desktop/
WebUI build. No hub deploy, no migration. Rollback = previous bundle.

## Open Questions

None blocking.
