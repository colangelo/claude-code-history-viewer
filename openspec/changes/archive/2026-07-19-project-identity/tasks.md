# Tasks: project-identity

## 1. Protocol + shared derivation

- [x] 1.1 Add `git_root_commit`, `git_remote_url`, `git_is_worktree`, `git_main_path` optionals (`#[serde(default)]`) to `IngestProject` in `crates/protocol/src/lib.rs`
- [x] 1.2 Add `normalize_remote_url` (credential-strip, scp→host/path, lowercase host, strip `.git`/trailing slashes) and `derive_identity_key` (`g:<root>|<remote>` / `g:<root>` / `r:<remote>`) to `crates/protocol` with unit tests (https/scp/credentialed/`.git`/trailing-slash/uppercase-host cases)

## 2. Daemon fingerprint capture

- [x] 2.1 New `crates/sync-daemon/src/git_fingerprint.rs`: guarded capture (only when `.git` marker present), 5s-timeboxed `git` subprocesses (`rev-parse --is-shallow-repository`, `rev-list --max-parents=0 HEAD` → lexicographically smallest root, `config --get remote.origin.url`), per-project failure isolation returning `Option<Fingerprint>`
- [x] 2.2 Wire into `to_ingest_project` / the scan loop (`sync.rs:109-123`): capture from `actual_path`, reuse `detect_git_worktree_info` result for `git_is_worktree`/`git_main_path`, normalize remote before send
- [x] 2.3 Tests with tempdir git repos: normal repo, no-remote repo, linked worktree, non-git dir, shallow clone (skip if `git clone --depth` unavailable), and capture-failure isolation (missing git binary path)

## 3. Hub schema + ingest

- [x] 3.1 `migrations/0003_project_identity.sql`: nullable `git_root_commit`/`git_remote_url`/`identity_key` TEXT + `git_worktree BOOLEAN NOT NULL DEFAULT false` on `projects`; index `projects_identity_key_idx`; `project_identity_aliases` (id, project_path UNIQUE, identity_key, created_by, created_at)
- [x] 3.2 Ingest upsert (`ingest.rs:128-159`): persist facts + hub-side re-normalize + derive `identity_key`; COALESCE semantics so absent facts never clobber stored non-nulls; re-derive on changed facts
- [x] 3.3 Ingest integration tests (runtime `sqlx::query*`, `SQLX_OFFLINE`-safe): fingerprint lands, absent-facts-retain, changed-remote re-derives, old-daemon payload (no fields) unchanged behavior

## 4. Hub identity reads + filter expansion

- [x] 4.1 Shared filter helper: parse `project` param — `identity:<key>` → member-path set (`projects.identity_key = key` ∪ `project_identity_aliases`), honoring `include_worktrees=false` (exclude path only when all its rows for the key are worktrees); plain values byte-compatible
- [x] 4.2 Apply to `/v1/sessions` (`browse.rs:111`) and `/v1/search` (`search.rs:76`); expose `identity_key` + `git_worktree` on `/v1/projects` rows (`browse.rs:42`)
- [x] 4.3 Apply to `/v1/journal/entries` (`journal.rs:377`) and journal search (`journal.rs:433`); `/v1/journal/pending` and POST untouched
- [x] 4.4 `GET /v1/identities` (members: path/providers/machines/worktree/last-activity; alias paths; suggestions: basename-match orphans + shared-root "related"); `POST /v1/identities/aliases` + `DELETE /v1/identities/aliases/{id}` on `Authenticated`, `created_by` audit
- [x] 4.5 Integration tests: identity expansion on all four read endpoints, alias create→included→delete→excluded round trip, worktree exclusion, fork (same root, different remote) never grouped, `identity:` reserved-prefix behavior, plain-filter byte-compat

## 5. Webapp

- [x] 5.1 `hubApi.ts`: `identity_key`/`git_worktree` on `HubProject`; `listIdentities`/`createAlias`/`deleteAlias`; `include_worktrees` + `identity:` filter plumbing
- [x] 5.2 Grouping util (pure, vitest-covered): rows → identity groups, display name = basename of most recent member, dimmed-suffix disambiguation on collisions, worktree labeling
- [x] 5.3 Browse sidebar (`ArchiveBrowser/index.tsx:487-504`): grouped entries, member inspection (machines/providers/paths, worktree labels), selection → `identity:` filter; ungrouped rows unchanged
- [x] 5.4 Journal dropdown (`JournalView.tsx:228-243`): identity options from grouped projects + path options for fingerprint-less; selection → `identity:` filter
- [x] 5.5 `worktreeVisibilityStorage.ts` (`cchv.archiveWeb.showWorktrees`, default true, `fontScaleStorage` pattern) + toolbar toggle wired to display filtering and `include_worktrees`
- [x] 5.6 Suggestion chip (link) + member unlink affordances with confirmation; i18n keys added across all 5 locales, `pnpm run i18n:validate` green

## 6. Quality gate + release prep

- [x] 6.1 Full gate: `pnpm tsc --build .`, `pnpm vitest run`, `pnpm lint`, `pnpm run i18n:validate`, `cargo test -- --test-threads=1`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`
- [x] 6.2 Docs: `docs/archive/deployment.md` rollout note (hub → daemons order, migration 0003 auto-runs); update `docs/agents`/CONTEXT pointers if needed
- [x] 6.3 Rebase onto `main` after cchv-v0.9.0 lands; resolve `ArchiveBrowser/index.tsx` overlap; merge; version bump 0.10.0 + `just sync-version` (release cut is a separate step on `main`)
