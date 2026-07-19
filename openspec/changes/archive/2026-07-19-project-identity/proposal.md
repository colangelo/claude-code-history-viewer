# Proposal: project-identity

## Why

Project identity in the archive is the literal `project_path` string. Moving a
repo to a new directory splits its history and journal timeline into two
unrelated projects; conversely, cross-machine folding only works today because
home layouts happen to match, and a same-named-but-unrelated directory (e.g. a
fork checked out elsewhere) would be wrongly folded by the journal's
path-keyed grouping. Git fingerprints (root commit + normalized remote URL)
give a stable, provider-independent identity that survives moves, clones, and
machine changes — and distinguishes true forks.

## What Changes

- The sync daemon captures a **git fingerprint** per project at scan time
  (root commit hash, normalized `origin` URL, worktree status) and sends it as
  additive `IngestProject` fields.
- The hub persists fingerprint facts on `projects`, derives a stable
  `identity_key`, and exposes it on read endpoints (migration `0003`).
- Read endpoints (`/v1/sessions`, `/v1/search`, `/v1/journal/entries`,
  `/v1/journal/search`) accept an **identity-scoped project filter**
  (`project=identity:<key>`) that the hub expands to all member paths —
  including manually aliased dead paths.
- A **manual alias layer** (`project_identity_aliases`) lets the user attach a
  moved-away (unfingerprint-able) path to an identity; reversible, view-level
  only — no ingested rows are ever rewritten. Basename-match **suggestions**
  surface candidate links.
- Git **worktrees are grouped as "related"**: labeled members of their main
  repo's identity, excludable via filter param and a persistent webapp toggle
  — never silently flattened.
- The webapp **projects dropdown and browse sidebar group by identity**,
  display basename-derived names (dimmed path suffix on collisions), and offer
  the alias/suggestion affordances.
- Journal entries stay keyed by `(entry_date, project_path)` (immutable
  provenance); identity grouping is a read-time lens. The distiller is
  untouched.

## Capabilities

### New Capabilities

- `project-identity`: git-fingerprint identity model — capture semantics
  (root commit, remote normalization, shallow/worktree handling), identity-key
  derivation, alias layer + suggestions, identity-scoped filter contract, and
  the identity/alias management endpoints.

### Modified Capabilities

- `history-sync-daemon`: project scan additionally captures git fingerprint +
  worktree facts (guarded, time-limited subprocess; failure never blocks sync).
- `archive-ingestion`: projects upsert persists fingerprint fields and derives
  `identity_key` (null never clobbers non-null); schema migration adds columns
  + alias table.
- `archive-search-api`: `/v1/projects` exposes identity fields; project
  filters on search/browse accept the `identity:<key>` scope with worktree
  exclusion.
- `journal-entries`: journal read endpoints accept the identity-scoped project
  filter; entry storage/distillation contract unchanged.
- `archive-journal-ui`: identity-grouped projects dropdown + browse sidebar,
  basename display names with collision disambiguation, worktree visibility
  toggle (localStorage), alias suggestion/unlink UI.

## Impact

- **Crates**: `protocol` (additive `IngestProject` fields + shared
  normalization/derivation fns), `sync-daemon` (new `git_fingerprint`
  module), `hub` (ingest upsert, browse/search/journal filters, new
  identity/alias endpoints, migration `0003`).
- **Webapp**: `src/services/hubApi.ts`, `ArchiveBrowser/index.tsx`,
  `JournalView.tsx`, new grouping util + `worktreeVisibilityStorage.ts`.
- **Wire/DB compatibility**: fully additive (serde defaults; no
  `deny_unknown_fields`; `ALTER TABLE ADD COLUMN`). Old daemons keep working
  against the new hub and vice versa. Rollout: hub first, then daemons.
- **Not touched**: distiller (`scripts/cchv-distill.py`), journal write path,
  desktop/Tauri surface, upstream-portable parser code in `history-core`
  (fingerprinting lives in the fork-owned daemon crate).
- **Version**: ships as `cchv-v0.10.0` (minor: new feature + migration).
