# Design: project-identity

## Context

Identity today is the literal `project_path`: DB unique key
`(machine_id, provider, project_path)` (`migrations/0001:41`), wire key
`(provider, project_path)`, journal fold `(entry_date, project_path)`
(`0002:64`), and every read filter (`browse.rs:138`, `search.rs:170`,
`journal.rs:392,448`). A moved repo splits everywhere; cross-machine folding
works only because home layouts coincide.

Useful existing pieces:

- `detect_git_worktree_info` (`crates/history-core/src/utils.rs:376`) already
  classifies Main/Linked worktrees from the `.git` marker — but the result is
  dropped at the wire (`convert.rs:137` never copies `git_info` into
  `IngestProject`).
- The daemon already resolves each project's real filesystem path
  (`sync.rs:109-113`, `actual_path`) — the right input for fingerprinting.
- Protocol is forward/backward compatible (all optionals `#[serde(default)]`,
  no `deny_unknown_fields`) — additive fields are free.
- The webapp has an established localStorage-settings pattern
  (`hubConfigStorage.ts`, `fontScaleStorage.ts`) and a single
  humanize-path rule (`JournalEntryCard.tsx:52-54`, Windows-tolerant
  basename).

Constraint from the deployment reality: hub and daemons upgrade
independently (hub swap via infra relay; daemons on m4m + ac-mbm5), so every
wire/DB change must be additive and order-independent.

## Goals / Non-Goals

**Goals:**

- One stable identity for a repo across moves, clones, machines, and
  providers, derived from git facts — with forks (same root, different
  remote) kept distinct.
- Worktrees grouped under their main repo's identity as labeled, excludable
  members.
- Manual, reversible alias layer for paths that can no longer be
  fingerprinted; suggestions assist, never act.
- Identity as a read-time lens: no ingested row is ever rewritten; removing
  an alias or shipping a rollback restores prior behavior exactly.

**Non-Goals:**

- No identity display-name override (derive from basename; alias table can
  grow a column later).
- No distiller/journal-write changes; entries stay `(entry_date,
  project_path)`-keyed.
- No desktop/Tauri surface changes; no upstream-portable code touched
  (`history-core` parsers unchanged — fingerprinting is fork-owned).
- No automatic merging of "related" identities (fork/remote-drift are
  suggestions only).

## Decisions

### D1. Fingerprint = facts on `projects` rows; identity = derived key. No identity table.

Alternative considered: a materialized `project_identities` table with FK
from projects. Rejected: it introduces stateful merge/split operations and a
second source of truth. Instead the daemon ships raw facts
(`git_root_commit`, `git_remote_url`, `git_is_worktree`), the hub derives a
deterministic `identity_key` string column per row, and "an identity" is
simply the equivalence class of rows sharing the key. Grouping stays a view;
the only persisted identity state is the explicit alias table.

Key format (normative in the spec): `g:<root>|<remote>` / `g:<root>` /
`r:<remote>`. Root+remote both in the key keeps forks distinct; remote-only
covers shallow clones; remote drift on a live repo just re-derives the key
in place on next upsert (dead rows that keep an old key are reachable via
the root-commit "related" suggestion).

### D2. Capture by `git` subprocess in `sync-daemon`, not `history-core`.

Alternatives: `gix`/`git2` crate (heavy dependency, and `history-core` is
the upstream-sync surface — keep it parser-only), or reading `.git`
internals by hand (root commit requires graph traversal; not feasible).
Subprocess (`/usr/bin/git` exists on macOS launchd's minimal PATH) runs only
when `detect_git_worktree_info` says there's a repo, with a hard timeout per
invocation and per-project failure isolation — the crush/aider
cloud-dir-walk wedge taught us that anything touching arbitrary user dirs
must be time-boxed and non-fatal. Three cheap invocations per project
(`rev-parse --is-shallow-repository`, `rev-list --max-parents=0 HEAD`,
`config --get remote.origin.url`), ~dozens of projects per pass: negligible.

`normalize_remote_url` + `derive_identity_key` live in `crates/protocol`
(shared wire semantics); the hub re-normalizes defensively rather than
trusting the daemon.

### D3. Server-side identity expansion via `project=identity:<key>`.

Alternative: client fetches member paths and issues N path-filtered queries.
Rejected: aliases live in the hub DB, so only the hub can resolve full
membership; N queries multiply latency and break unified search ranking.
The `identity:` prefix rides the existing `project` param, so all four read
endpoints gain the feature with one parsing helper and a
`project_path IN (members ∪ aliases)` expansion. Plain filters stay
byte-compatible; `identity:` is a reserved prefix (absolute paths never
collide with it).

`include_worktrees=false` excludes a member path only when *every* project
row binding that path to the identity is a worktree (a path that is a main
checkout anywhere stays included).

New grouping SQL uses runtime `sqlx::query*` (the `journal.rs:11-15`
convention) to stay `SQLX_OFFLINE`-safe.

### D4. Identity also folds providers.

Same directory, multiple assistants → N project rows (one per provider), all
fingerprinting identically → one identity. This is intended: the repo is one
project; provider stays visible as member provenance and as the existing
`provider` filter. No schema work needed — it falls out of D1.

### D5. Alias writes accept the read-auth principal.

Aliases are reversible view metadata, so the write bar matches read access:
`Authenticated` extractor (machine bearer *or* trusted Tailscale identity),
with the principal recorded in `created_by` for audit. Alternative (bearer
machine-token only) would make the webapp — the natural place to click
"link" — unable to manage aliases in tokenless Tailscale mode. Accepted
risk mirrors the documented read-auth posture (loopback header spoof by
local same-user processes) and the blast radius is a reversible alias.

### D6. UI grouping is client-side over `/v1/projects` identity fields.

The sidebar/dropdown group rows by `identity_key` in a small shared util
(display name = basename of most recently active member, dimmed
disambiguating suffix on collisions — generalizing the
`JournalEntryCard.tsx` basename rule). Server-side grouped responses were
rejected: the client already holds all rows, and display grouping is a view
concern; the server's job is filter expansion (D3), where it's authoritative.
Worktree toggle follows `fontScaleStorage.ts` as
`worktreeVisibilityStorage.ts` (`cchv.archiveWeb.showWorktrees`, default
true).

### D7. Root-commit determinism and edge cases.

Multiple roots (merged unrelated histories): lexicographically smallest —
deterministic regardless of HEAD or merge order. Shallow clones: `rev-list`
would return graft boundaries, i.e. a *false* root that differs per clone
depth — detect via `rev-parse --is-shallow-repository` and fall back to
remote-only fingerprint. No remote and shallow: no fingerprint. Credentials
in remote URLs are stripped before anything leaves the daemon (never store
tokens), and the hub's defensive re-normalization strips again.

## Risks / Trade-offs

- [Remote drift / repo renamed on the host] → live rows re-derive in place;
  stranded dead rows surface via the root-commit "related" suggestion, fixed
  with one alias click.
- [git subprocess hangs on network/cloud-backed dirs] → hard timeout +
  per-project isolation; worst case a project simply stays ungrouped
  (observed failure mode: `providers_exclude` precedent).
- [Same-basename fork suggestion tempts a wrong link] → suggestions are
  advisory chips; a wrong alias is one DELETE away and never mutated any
  ingested data.
- [Journal day split on a mid-day move] → both path-entries returned under
  one identity heading; cosmetic duplication for one day, provenance intact.
- [`bool_and`-style worktree exclusion has a blind spot] — a path worktree
  on one machine, main on another stays included → acceptable: inclusion is
  the safe default.
- [Webapp older than hub or vice versa] → all changes additive; unknown
  fields ignored, `identity:` filter only sent by a webapp that knows it.

## Migration Plan

1. Migration `0003` (auto-run by `MIGRATOR` at hub startup): `ALTER TABLE
   projects ADD COLUMN git_root_commit/git_remote_url/identity_key TEXT,
   git_worktree BOOLEAN NOT NULL DEFAULT false`; index on `identity_key`;
   `CREATE TABLE project_identity_aliases`.
2. Deploy hub first (accepts + persists new fields; old daemons unaffected).
   Staging → infra relay → binary swap per `docs/archive/deployment.md` §2b.
3. Deploy daemons (m4m, ac-mbm5) — next scan pass backfills fingerprints on
   all live projects via the normal upsert; no data migration, no config
   change.
4. Webapp ships in the same release (static bundle via hub `static_dir`);
   cache split (immutable assets / no-cache HTML) already handles rollout.
5. Rollback: previous hub binary ignores the new columns; aliases persist
   harmlessly; daemon rollback just stops sending facts (COALESCE keeps
   stored ones).

## Open Questions

- None blocking. Deferred (noted in Non-Goals): identity display-name
  override; richer fork-relation UI beyond the "related" suggestion.
