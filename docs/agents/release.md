# Version management and releases

> Moved word for word from `AGENTS.md` on 2026-10-30 (ac's yes, asks row a1030-39), which links here.

## Version Management

The fork ships the **web viewer + archive stack** (static archive webapp, WebUI
server, hub, sync-daemon) — **no desktop distribution**. It owns a single
version line, **`cchv-vX.Y.Z`**, decoupled from upstream's `v1.x`. Upstream's
`v1.x` tags are fetched for the parser supply chain but are **not** ours; on an
upstream sync, keep our version. The line is `0.x` (pre-stable dogfood tier).

Version history (see `git tag -n 'cchv-v*'`): `cchv-v0.1.0` archive MVP ·
`v0.2.0` daemon hardening · `v0.3.0` hub API DX + archive viewer UI · `v0.4.0`
static webapp + hub static hosting · `v0.5.0` Tailscale-identity read-auth ·
`v0.5.1` SPA cache split (current, live on m4m).

### Single Source of Truth

**`package.json`** `version` is the source of truth. `just sync-version`
propagates it to the Rust workspace and the distiller:

```
package.json (version)
    ↓ just sync-version
├── Cargo.toml  [workspace.package] version   ← every crate inherits it
│                                               (version.workspace = true);
│                                               the hub reports it on /v1/healthz
└── scripts/cchv-distill.py  DISTILL_VERSION  ← the distiller announces it at every
                                               tick (it is deployed as an installed
                                               COPY — #40). Marker-anchored; a missing
                                               marker aborts the sync before any write.
```

Not a target: `Cargo.lock` — refreshed by a `cargo` invocation, guarded by Guard 2
below. Consequence of the distiller target: **every release changes
`scripts/cchv-distill.py` by one line**, so the installed copy reads one release
behind until infra reinstalls it; a version-only skew shows as a one-line
`git diff <old-tag> <new-tag> -- scripts/cchv-distill.py`, and the blob id the
distiller also reports is the tie-breaker (`docs/archive/deployment.md` §3c).

### Version Bump Guide

```bash
# edit package.json "version", or bump the number (no npm publish):
npm version <patch|minor|major> --no-git-tag-version   # e.g. 0.5.1 → 0.6.0
just sync-version                                       # propagate (required)
```

SemVer: bug fix → patch, new feature → minor, breaking → major.

### Release Process

> **Invoke the `cchv-deploy` skill** (`~/_sync/dev/CONTEXT/SKILLS/cchv-deploy/`,
> linked on every Mac) before cutting a release or relaying a deploy. It carries
> the order of operations, what a relay must contain, and the traps — including
> the two this section does not: `Cargo.lock` needs a `cargo` invocation because
> `just sync-version` does not touch it, and **the webapp ships with every
> release** because the bundle embeds the version the header chip renders. The
> ceremony itself stays in `docs/archive/deployment.md` §2b/§2c, which infra
> co-owns.

#### Phase 1: 품질 검증 (Quality Gate)

릴리즈 전 **모든 검증을 통과**해야 한다. 하나라도 실패하면 수정 후 재검증.

```bash
# ===== Frontend 검증 =====
pnpm install                    # 의존성 동기화 (lockfile 불일치 방지)
pnpm tsc --build .              # TypeScript 빌드 체크 (CI와 동일)
pnpm vitest run --reporter=verbose  # 프론트엔드 테스트
pnpm lint                       # ESLint (no-explicit-any 등)
pnpm build                      # webui-server가 dist/를 임베드 — 새 worktree에서는 이게 없으면 clippy --all-features 실패

# ===== Backend 검증 =====
cd crates/viewer && cargo test -- --test-threads=1 && cd ../..  # Rust 테스트 (단일 스레드 필수)
cd crates/viewer && cargo clippy --all-targets --all-features -- -D warnings && cd ../..  # Rust 린트
cd crates/viewer && cargo fmt --all -- --check && cd ../..      # Rust 포맷 체크

# ===== i18n 검증 =====
pnpm run i18n:validate          # 5개 언어 키 동기화 확인 (en, ko, ja, zh-CN, zh-TW)
```

**주의사항:**
- `cargo test`는 반드시 `--test-threads=1`로 실행 (settings 테스트가 `env::set_var("HOME")` 사용)
- `pnpm install` 생략 시 lockfile과 node_modules 불일치로 빌드 실패 가능
- lint에서 `@typescript-eslint/no-explicit-any` 에러 발생 시 `as unknown as TargetType` 패턴 사용

#### Phase 2: decide the version

```bash
# commits since the last cchv release (glob scoped to OUR line, not upstream v*)
git log "$(git tag --list 'cchv-v*' --sort=-version:refname | head -1)"..HEAD --oneline
```

SemVer: bug fix → patch (0.5.1 → 0.5.2), new feature → minor (0.5.1 → 0.6.0),
breaking → major.

#### Phase 3: bump, tag, push (on `main`)

```bash
npm version <version> --no-git-tag-version   # e.g. 0.6.0 (no npm publish)
just sync-version                            # package.json → workspace + distiller
cargo check -q -p hub                        # REFRESHES Cargo.lock. sync-version does NOT.
                                             # The skill's Phase 2 has carried this line all
                                             # along; the block did not, and the block is the
                                             # copy-pasted half — the `af2c3128` divergence
                                             # pointing the other way. Guard 2 below is the
                                             # backstop for the day it is skipped anyway.
pnpm tsc --build . && pnpm vitest run        # re-check after sync

# Census the WORKTREE before staging. A Syncthing conflict COPY OF A TRACKED FILE is a
# different class from the conflict ref below: it is an ordinary untracked file, and it
# reaches the PUBLIC fork through the STAGING step — not through a stray `--all` on the
# push. Two independent defences; keep both, because neither covers the other:
#   1. Stage EXPLICIT PATHS (below) so the release commit cannot sweep one up at all.
#      The cchv-deploy skill's Phase 2 has always said this; this block said `git add -A`,
#      and the block is the half that gets copy-pasted, so the weaker rule won. Fixed
#      2026-08-16 — the same reach failure infra recorded in CONTEXT `PATTERNS/docs-okf.md`
#      § Every placement has a reach, here spanning two documents instead of one.
#   2. Run the census anyway. Explicit staging makes a conflict copy harmless to THIS
#      commit but silent — and its danger is precisely going unnoticed.
# We are better off here than infra was, and it is worth knowing exactly why: cchv does
# NOT `.gitignore` the pattern, so a copy shows up as `??` (measured 2026-08-16 on a
# throwaway file: `check-ignore` exit 1, `git status` lists it, `add -A --dry-run`
# stages it). **Never add the pattern to `.gitignore`**: that suppresses the only
# routine signal for a class whose whole danger is going unnoticed. It is how infra sat
# on a live 146 KB copy for five days with four checks reporting clean (infra 2d87f32).
find . -path ./.git -prune -o -name "*.sync-conflict-*" -print   # expect: nothing
# Exactly what a version bump touches — four files verified against `chore(release):
# cchv-v0.18.1` (b8f69d3a), plus `scripts/cchv-distill.py` since build-identity-surfaces
# (#40, 2026-08-21: the distiller's DISTILL_VERSION is sync-version's fifth target),
# minus `src-tauri/tauri.conf.json` since the web-only cut (#23) deleted it. Leaving a
# deleted path here is not harmless: `git add` refuses the WHOLE command on a pathspec
# that matches nothing, so the release commit would carry no bump at all.
# Anything else that belongs in the release gets committed deliberately BEFORE this
# point, never swept in here.
git add package.json Cargo.toml Cargo.lock scripts/cchv-distill.py

# Explicit staging fails closed in the WRONG direction the day `sync-version` grows a
# new target: the list silently drops it, and the worktree census above says nothing —
# it hunts UNTRACKED strays. This is no longer hypothetical: the fifth target landed
# 2026-08-21, and the line above grew with it in the same commit — which is the only
# reason Guard 1 did not have to fire. Two guards close the case where it does not, and
# they are complements, not alternatives: Guard 1 catches a bump target that CHANGED
# and went unstaged, Guard 2 one that was never TOUCHED. Both compose with explicit
# staging; neither reaches for `-A`.
#
# Guard 1 — infra's (measured by them, re-measured here 2026-08-16 in a throwaway repo:
# 5 tracked targets / recipe stages 4 → rc=1 naming `fifth.json`; all 5 staged → rc=0; an
# untracked `*.sync-conflict-*` copy → does NOT fire, `??` is invisible to `git diff`).
# Placed BEFORE the commit, not after as infra ran it: detection is identical (measured
# both), but here the fix is `git add <file>` rather than `git commit --amend`.
# It also fires on any UNRELATED tracked edit sitting in the tree — a peer's in-flight
# Syncthing edit arriving worktree-first. That is noise in the fail-closed direction:
# stopping a release to look at a peer's edit in your tree is the correct answer anyway.
git diff --quiet || { echo "unstaged tracked changes at release time:"; git diff --name-only; exit 1; }

# Guard 2 — the stale `Cargo.lock`, which the skill calls the recurring miss. Skip the
# `cargo` invocation above and the lock keeps the OLD version UNMODIFIED, so Guard 1 is
# silent and the release ships a lock disagreeing with every other bump target.
# Workspace crates are derived STRUCTURALLY (a Cargo.lock entry with no `source =` field),
# never a hardcoded name list — a seventh crate is covered the day it lands, which is the
# whole objection this guard answers. `END{emit()}` is load-bearing: Cargo.lock ends with
# `]`, not a blank line, so a `/^$/`-only trigger drops the final package.
# Do NOT swap in the one-liner `grep -c "^version = \"$V\"" Cargo.lock`: measured
# 2026-08-16 it FALSE-PASSES — cookie, glib-sys, png and gio-sys all sit at 0.18.1 today,
# so it reports coverage while every workspace crate is stale. A guard that false-passes
# is worse than none: it reads as the control. (Same shape as infra's `git add <dir>`.)
V=$(node -p "require('./package.json').version")
stale=$(awk '
  function emit(){ if (n != "" && s == 0) print n, v }
  /^\[\[package\]\]/ { emit(); n=""; v=""; s=0; next }
  /^name = / {n=$3} /^version = / {v=$3} /^source = / {s=1}
  END { emit() }' Cargo.lock | grep -v "\"${V}\"$")
[ -z "${stale}" ] || { echo "stale Cargo.lock — run the cargo invocation, then re-stage:"; echo "${stale}"; exit 1; }

git commit -m "chore(release): cchv-v0.6.0"
SHA=$(git rev-parse HEAD)   # capture NOW — the publication proof below asserts this
                            # value, not `HEAD`, and nothing may move the ref first.
git tag -a cchv-v0.6.0 -m "cchv-v0.6.0"

# Now census the REF list, before publishing anything. `.git` here is Syncthing-shared,
# so a peer session's collision leaves a ref under `refs/heads/` — a LIVE branch named
# `main.sync-conflict-…`, which a clean `git status` never reports and which `origin`
# would carry to the PUBLIC fork. `for-each-ref` reads loose AND packed refs; a `find`
# scoped to `.git` sees the ref only until `gc --auto` packs it, and then reports clean
# while the branch is still on the push list (measured here 2026-08-16 — infra fd5ab18).
# That finding RE-SCOPES `find` — off `.git`, onto the worktree — it does not retire it.
# Read one clause short, as "`find` is the wrong tool", and you delete the only census
# that sees the file-shaped class above; infra's tree was cleared by exactly that
# misreading three hours before they found the copy by hand. Two censuses, both
# required, neither covering the other's class.
# Quarantine a non-ancestor tip to `refs/backup/`, never delete it.
git for-each-ref --format='%(refname)' | grep -i conflict   # expect: nothing under refs/heads/
git push internal main && git push internal cchv-v0.6.0
git push origin  main && git push origin  cchv-v0.6.0

# Positive proof the release actually left this Mac, on EVERY remote you pushed.
# `.git` here is Syncthing-shared, so a peer session can rewind your ref — and then
# `git push` prints "Everything up-to-date", byte-identical to a genuine no-op, while
# `git log -1` agrees with it (the ref and reflog are what got replaced). Never read
# the absence of a push error as publication.
# CONTEXT `PATTERNS/git.md` § the push-side twin of the reflog census.
#
# Two assertions per remote, because they cover different failures. The tag check
# queries the hub BY NAME, so no local ref can answer for it — but it says nothing
# about `main`: a swap landing mid-release still carries your objects with the tag, so
# the tag verifies green while the hub's `main` lacks the release commit. Assert the
# branch separately, by the sha you MEANT.
for R in internal origin; do
  git ls-remote --tags "$R" cchv-v0.6.0 | grep -q . \
    || { echo "tag NOT published on $R"; exit 1; }      # empty = nothing was published
  git fetch "$R" main -q || { echo "fetch failed on $R — cannot answer"; exit 1; }
  # `--is-ancestor` answers on the EXIT CODE, and non-zero carries two different
  # answers: 1 = "no, not an ancestor", 128 = "I could not answer, that object does
  # not exist". `||` and `2>/dev/null` collapse them, so a FABRICATED sha takes the
  # same branch as a true negative and reads as a verified "not published". Measured
  # 2026-08-21; it is how a sha that resolved to nothing became a published fact.
  git merge-base --is-ancestor "${SHA}" "$R"/main; rc=$?
  case $rc in
    0) echo "published on $R: ${SHA}" ;;
    1) echo "main NOT published on $R"; exit 1 ;;
    *) echo "cannot answer on $R (git exit $rc — does ${SHA} exist?)"; exit 1 ;;
  esac
done
```

**Assert both remotes — the push ORDER is not what holds this up.** This block used to
assert `origin` alone, on the argument that `origin` is pushed last so a swap landing
anywhere earlier leaves `origin/main` short of `${SHA}`. That argument is true and it is
not enough, because **it assumes both pushes succeed.** The four push statements above
are separate commands with no `set -e`: a failed `internal` tag push does not stop the
`origin` line, `origin` goes green, and nothing ever asks about `internal`. Ordering can
cover a *swap*; it cannot cover a *failure*. So assert each remote you pushed, and the
order becomes free. Generally: a single-remote publication proof is sound only when it
names the remote you push **last** *and* nothing between the pushes can fail.

This correction came from the `cchv-deploy` skill, which is the **other copy of this
recipe** — and the two have drifted in both directions, alternating: `af2c3128` fixed
explicit staging in *this block* after the skill had it; the `cargo check -q -p hub`
line above went the other way (the skill carried it first, this block picked it up);
and the both-remotes proof went the skill's way again. **Which half is behind does not
carry over**, so when either changes, diff them as whole documents rather than
spot-checking the line that just burned you — and check the claim you are about to
write against the file in front of you, not against the skill's account of it. The
skill still says this block "had never carried" the `cargo check` line; line 317 says
otherwise, which is the same staleness one document keeping notes on another always
develops.

**Assert `${SHA}`, never `HEAD`.** `HEAD` is a symref *through* `refs/heads/main` —
the exact file a peer's branch swap renames aside — so after a swap it resolves to
the peer's commit, and `--is-ancestor HEAD origin/main` prints *"main published"*
whenever the peer pushed, while your release commit sits unpublished on the conflict
ref. It misfires precisely in the two-session burst that causes the swap. Derived
by infra from the ref mechanics (`4a733b5`, CONTEXT `PATTERNS/git.md` §
Syncthing-shared `.git`) and **measured here 2026-08-16** on a staged swap: the
`HEAD` form printed *"main published"* while the release commit sat on the conflict
ref, the `${SHA}` form correctly failed, and in the tag variant `ls-remote --tags`
returned the tag while the hub's `main` lacked the release commit.

#### Phase 4: CI + deploy

Pushing a `cchv-v*` tag runs `.github/workflows/server-release.yml`, which
publishes a GitHub Release with the static webapp bundle (`cchv-webapp.tar.gz`)
only. The per-platform WebUI server binaries are **dispatch-only** (nothing
consumes them today): `gh workflow run server-release.yml --ref cchv-vX.Y.Z`
builds and attaches them to that tag's release on demand.

For a **tagged** release the workflow also builds and attaches the hub binary
`cchv-hub-<version>-aarch64-apple-darwin` + `.sha256` (macos-14 job, since the
v0.11.2/v0.12.0 CI change). The always-on **m4m hub IS deployed from that
asset**: infra downloads it from the release, verifies the digest, stages it in
`~/.config/cchv/staging/`, and does the codesign-aware swap per
`docs/archive/deployment.md` §2b. The local-build → scp path is only the
fallback when no release asset exists. Relay the deploy to home-network (infra)
— the swap runs on m4m, not from your Mac.

```bash
# NB: bare `gh` defaults to the UPSTREAM repo (jhlee0409) — always -R the fork
# for our cchv-v* line, or you'll query upstream's v1.x releases and see nothing.
FORK=colangelo/claude-code-history-viewer

# Check EVERY workflow on the TAG's sha, not just the release build. "CI green"
# scoped to server-release.yml was wrong twice on 2026-08-21, the same way both
# times: the release workflow was green and the TAG was not. It is easy to cut a
# tag minutes before an unrelated fix lands on main, and then main is green while
# the tag stays red forever — a red the next reader has to re-triage.
TAGSHA=$(git rev-list -n1 cchv-vX.Y.Z)
gh run list -R "$FORK" --commit "$TAGSHA" --json workflowName,conclusion \
  --jq '.[] | "\(.conclusion // "running")  \(.workflowName)"'
# Anything not `success` is either fixed BEFORE the tag or stated explicitly in
# the relay with the evidence that it does not reach the artifact. Do not report
# "CI green" from one workflow — say which.
gh run list -R "$FORK" --workflow=server-release.yml --limit=1
gh release view cchv-v0.13.0 -R "$FORK"   # expect 3 assets: hub bin + .sha256 + webapp
```

#### Troubleshooting

| Problem | Cause | Fix |
|---------|-------|-----|
| CI pnpm version clash | `pnpm/action-setup` `version` vs `package.json` `packageManager` | drop `version` in the workflow (auto-detected) |
| `cargo test` flaky | `env::set_var("HOME")` is process-global → parallel race | `--test-threads=1` |
| Duplicate release | manual `gh release create` + workflow auto-create | let `server-release.yml` own it |
| Modules not found after `pnpm install` | lockfile ↔ node_modules drift | `rm -rf node_modules && pnpm install` |

### Desktop app (removed by the web-only cut, #23)

There is no Tauri desktop app any more — not as a distribution and, since the
web-only cut (`openspec/changes/web-only-cut/`, Gitea #23), not as a dependency
either. `crates/viewer` is a plain Rust crate whose binary does two things, both
dispatched from `lib.rs::run()`: `--export` (headless session export) and
`--serve` (the WebUI server). With neither flag it prints usage and exits 2.

- `Cargo.toml`: `default = ["webui-server"]`, no `tauri*` crate at all;
  `cargo tree -i tauri --target all` matches nothing. No `build.rs`,
  `tauri.conf.json` or `capabilities/`.
- The old `#[tauri::command]` functions are plain `async fn`s the axum handlers
  call; `commands/mod.rs` allows `clippy::unused_async` because the handler
  macros await every command uniformly.
- Gone with the desktop: the updater (Rust + `useUpdater`/update modals + the
  `update` i18n namespace + `update-flow-tests.yml`), the `--session` startup
  hint and its session picker, reveal-in-Finder, the native folder pickers
  (typing a path stays), the WSL settings section (the server-side WSL scan
  stays), and the macOS window chrome.
- The Linux CI jobs no longer install GTK/WebKit: nothing in the graph needs a
  system library (no `glib-sys`, `webkit2gtk`, `soup3`, `openssl-sys`).
- **Logging:** `tauri-plugin-log` was locked but never registered, so `--serve`
  had no `log` backend before the cut either; its `log::` calls stay silent and
  user-facing output goes through `eprintln!`. Adding a logger is a separate
  decision, not part of the cut.

The frontend has no `isTauri()` any more. Every former branch kept its web half
(`fetch` for `api()`, `localStorage` for `storageAdapter`, Blob downloads for
file saves, an anchor click for `openExternalUrl`, SSE for the file watcher).

### What CI builds, and who consumes it

Checked 2026-07-26. Useful when deciding whether a red check is worth fixing.

| Artifact | Built by | Consumer |
|---|---|---|
| `cchv-webapp.tar.gz` | `server-release.yml`, every `cchv-v*` tag | **Us** — infra swaps it into m4m's `static_dir`; the live archive browser |
| `cchv-hub-<v>-aarch64-apple-darwin` + `.sha256` | `server-release.yml`, every tag (macos-14) | **Us** — becomes `~/.local/bin/cchv-hub` on m4m (`docs/archive/deployment.md` §2b) |
| 4× WebUI server binaries (`crates/viewer --features webui-server`) | `server-release.yml`, **dispatch-only** | **Nobody today.** Free unless dispatched |

`crates/viewer` therefore ships to no one — but it is **not dead code**. It is the
local CLI the `cchv-find` skill §3 drives, built from source on demand:
`--export <id|path> --format html` and `--serve` both verified working
2026-07-26. So `rust-tests.yml` guards a real local tool, just not a shipped
artifact — weigh it accordingly.

Test workflows by what they guard: `archive-tests.yml` → the crates we actually
ship (history-core, protocol, hub, sync-daemon). `frontend-tests.yml` → the
webapp we ship. `rust-tests.yml` → the local-only CLI.

Known dead weight in `rust-tests.yml`: the **Benchmarks** job uploads nothing —
`cargo bench --no-run` never produces criterion output, and its
`src-tauri/target/criterion` path went stale when the crates extraction moved
`target/` to the repo root (the run annotation reads *"No files were found with
the provided path"*). It only proves benches compile.

**`security-audit.yml` was red from 2026-08-10 to 2026-08-21 on one unreachable
advisory. It is green now — landed at `2a318594`.** Kept because the *shape* recurs
and because four release tags carry the red mark permanently.

**⚠️ `cchv-v0.20.1` (`ac3945ec`) is the last tag with the red mark, and it always will
be.** The ignore landed **one commit after** that tag, so the Phase 4 check above —
`gh run list --commit <tag-sha>` — reports `Security Audit  failure` for it, forever, and
every future reader will re-triage it. The same is true of v0.18.1, v0.19.0 and v0.20.0.
It does not reach the deployed artifact: the advisory is `src-tauri`-only (see the chain
below), the hub binary cannot contain it under any feature selection, and infra verified
that independently with `strings` on the shipped asset before swapping. **A red
`Security Audit` on any tag at or before `ac3945ec` is expected and already triaged; on a
tag after it, it is new and real.**

- The run reports **`error: 1 vulnerability found!` plus 30 allowed warnings** —
  the warnings do not fail it. The single vulnerability is **RUSTSEC-2026-0235**,
  `rkyv 0.7.46`, out-of-bounds reads on archives containing `Rc`/`Arc`.
- **`rkyv` is never compiled.** It is an *optional* dependency of
  `rust_decimal 1.37.2` whose feature is not enabled. `cargo tree -i rkyv
  --target all` prints nothing, while `cargo tree -i rust_decimal --target all`
  prints the whole chain: `rust_decimal ← byte-unit 5.1.6 ← tauri-plugin-log
  2.8.0 ← claude-code-history-viewer` (`src-tauri`). The chain is real up to the
  optional edge and stops there.
- Note where the chain lands. `tauri-plugin-log` is one of the unconditional
  desktop deps from *Desktop app (retired…)* above, so this advisory cannot
  reach the **hub binary we actually deploy** under any feature selection — not
  merely under today's. It is a `src-tauri`-only lockfile entry, and the
  web-only-cut deletes it outright.
- `cargo audit` scans `Cargo.lock`, not the resolved feature graph, so a locked
  entry is enough to fail it. `.cargo/audit.toml` exists for precisely this and
  already carries three such entries (`rsa` via `sqlx-mysql`, two `quick-xml`
  via `plist`) — the `rsa` one proves unreachability with the same
  `cargo tree -i … --target all` probe used above. **`"RUSTSEC-2026-0235"` was added
  with that reasoning at `2a318594`** — the repo's own convention, not a new judgment
  call. It had been left unmade only because `.cargo/` is permission-gated for agents
  and an unattended run must not talk its way past that gate; an attended session
  pasted the block that had been parked on #11 (comment 7159) for exactly that reason.
  Way out for the entry: `rust_decimal` moving to `rkyv` 0.8, or the web-only-cut,
  whichever lands first.
- **The ignore cannot outlive its justification silently.** `security-audit.yml` now
  fails the build if any *absence-based* entry (`rsa`, `rkyv`) enters the build graph.
  `quick-xml` is deliberately excluded from that guard — it **is** in the graph
  (`plist → tauri/os_info`) and its ignore rests on the input being Tauri's own
  `Info.plist`, not on absence. A name in that list asserts absence; verify with
  `cargo tree -i <crate> --target all` before adding one.
- **Since the web-only cut (#23), `rkyv`, `rust_decimal`, `quick-xml` and `plist` are
  not in `Cargo.lock` at all**, so `RUSTSEC-2026-0235`, `-0194` and `-0195` in
  `.cargo/audit.toml` ignore advisories for crates that no longer exist, and the
  absence guard lists `rsa` only. Removing the three dead entries is an attended edit
  (the file is permission-gated); until then they are inert, not wrong.
