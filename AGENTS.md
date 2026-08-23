# AGENTS.md

The single guidance file for this repo — house conventions for agents *and*
project/code guidance, merged 2026-07-26. **`CLAUDE.md` is a symlink to this
file**, so either path resolves here; edit this one. (Nested `CLAUDE.md` files
inside subdirectories are claude-mem artifacts, gitignored via
`**/*/CLAUDE.md`, and are not authoritative — see `docs/agents/domain.md`.)

If the user's prompt starts with “EP:”, then the user wants to enhance the prompt. Read the PROMPT_ENHANCER.md file and follow the guidelines to enhance the user's prompt. Show the user the enhancement and get their permission to run it before taking action on the enhanced prompt.

The enhanced prompts will follow the language of the original prompt (e.g., Korean prompt input will output Korean prompt enhancements, English prompt input will output English prompt enhancements, etc.)

## Principal

Use pnpm Package Manager.

`gemini`/`codex` CLIs are **opt-in adversarial reviewers only** — invoke them
solely with explicit user approval (see `.claude/commands/pr-review-check.md`),
never as a mandatory first step.

가독성이 높은 설계 추구
예측 가능성이 높은 설계 추구
높은 응집도 설계 추구
낮은 결합도 설계 추구

## Working conventions

### House conventions & skills

Cross-project conventions and skills live in **`~/_sync/dev/CONTEXT/`** (progressive
disclosure — read an index, drill only what you need): `PATTERNS/index.md` for "how we
do X here" (Justfiles, git remotes, containers, secrets, docs, backlog, CI, second-loop,
…), `SKILLS/README.md` for actionable skills (read the matching `SKILL.md` and follow
it). Check PATTERNS before inventing a convention; check SKILLS before improvising a
procedure.

### Specs: OpenSpec, not ad-hoc design docs

**This is a brownfield repo, so every non-trivial change is specced through
OpenSpec** — `openspec new change "<kebab-name>"`, then the artifacts under
`openspec/changes/<name>/` (`proposal.md`, `design.md`, `tasks.md`, plus the
`specs/<capability>/spec.md` deltas). `tasks.md` *is* the implementation plan; don't
write a second one somewhere else. Archive with the openspec archive flow when the
change lands.

The trap: general-purpose planning skills (Superpowers `brainstorming` →
`writing-plans`, etc.) default to writing a design doc to
`docs/superpowers/specs/YYYY-MM-DD-<topic>-design.md` and to their own plan format.
**That default does not apply here** — the house rule wins, and a spec that lands
outside `openspec/` is invisible to the delta/archive audit trail the workflow
exists for. Use those skills for their *thinking* (the questions, the approach
comparison) and then land the result as OpenSpec artifacts. The two design docs that
already sit in `docs/superpowers/specs/` are historical, not a precedent to follow.

### Evidence over prose

**Before acting on a claim in this file that some subsystem is gone, retired, or
unused, check the dependency graph and the entrypoint — not the prose.** A doc
sentence is a hypothesis; `Cargo.toml`, `Cargo.lock`, and `lib.rs::run()` are the
evidence.

This rule exists because the *Desktop app (retired…)* section below asserted, until
2026-07-26, that `src-tauri` "now builds solely as the WebUI server". It does not,
and the false sentence propagated into reasoning about CI. Same for a red check:
know what a workflow guards before spending effort turning it green — see *What CI
builds, and who consumes it* for the current artifact/consumer table.

### Secrets

Machine reads default to **OpenBao** — `bao kv get kv/<path>` (`BAO_ADDR` is
machine-wide; auth is ac's daily `bao login -method=oidc`, 12 h token). If the
token is missing/expired, fall back to `op read` and tell ac to re-login.
1Password stays the human vault and the fallback (vault `AC-DevOps`). Never
commit/inline a secret — reference a 1P item title or a bao kv path. Need a
seed / AppRole / ACL grant? Relay message or `agent-relay` issue to
home-network. Canonical: home-network `docs/secrets-standard.md`; portable
copy: CONTEXT `PATTERNS/secrets.md`.

Repo-specific: the cchv secrets ARE seeded in OpenBao (home-network#17, done
2026-07-05) — `kv/infra/cchv/pg1` (hub DB creds) and `kv/infra/cchv/hub-tokens`
(per-machine hub tokens). The **hub DB password is moving out of `kv/`** to a
credential bao owns and rotates (`database/static-creds/cchv-svc`, 30d —
home-network #31): the launcher reads it with `bao_static` and keeps the `kv/`
mirror only as a cutover fallback, so a rotation is picked up by relaunching the
hub. The tokens stay in `kv/`. The always-on archive jobs (daemon + hub) are
**bao-first** via `scripts/cchv-launch.sh` and the AppRole `cchv-daemon`
(creds file `~/.config/cchv/bao-approle`, from 1P item
`openbao - cchv-daemon approle`); `op read` is the fallback, a last-known-good
rendered config the floor. See `docs/archive/deployment.md` §3b. Flipped on
m4m 2026-07-05; ac-mbm5's daemon still runs the old static config — flip it
with the §3b per-machine steps next time an attended session is on ac-mbm5.

### History archive ops

Archive deployment: `docs/archive/deployment.md`. Recovering history older
than Claude Code's ~30-day local retention from Time Machine backups (any
machine, incl. retired ones via their TM disk): `docs/archive/timemachine-backfill.md`
(`just tm-backfill --list` to see what's recoverable).

### Agent relay

This repo is a **`nats`-variant relay participant** (since 2026-07-12): messages
travel the NATS JetStream work queue on `bus.internal` — **no committed
`agent-relay/` surface** (this tree is a public fork; constellation content must
never enter its history; pre-flip traffic remains in git history only). `auto`
messages are handled by the always-on relay supervisor; `interactive` ones are
drained by an attended session (`/check-relay`). Tracked, human-visible asks still
arrive as Gitea issues labelled `agent-relay`. Canonical spec + registry:
home-network `agent-relay/AGENTS.md` (portable: CONTEXT `PATTERNS/agent-relay.md`);
send with home-network `tools/relay-send`.

**Checkpoint by committing — an uncommitted fix is not partial progress, it is
nothing.** A handler here works a tree whose `.git` *and* worktree are
Syncthing-shared, so written-but-uncommitted work is not merely unsaved: it sits
where the replicator can land the peer's older copy over it, and the displaced
bytes survive only as a `*.sync-conflict-*` file nobody is looking for. Measured
on one thread: **three** consecutive handlers of msg `95ed910c` were killed
mid-flight having each already *written* the fix, and from the sender's side
nothing had landed and nothing said so — the audit trail for that msg-id carried
the `sent` event and literally nothing else (infra, 2026-08-16). We were bitten
by the same shape one round earlier: `e610deb4` had to fold in exactly such an
orphan from a prior session on the same thread. So **"unpushed is unpublished"
has a shorter-fused sibling — uncommitted is unwritten**, and its fuse is a
killed process rather than a missing push. Commit at each phase boundary, not at
the end; the `RELAY-OUTCOME` line is the last thing that runs and is therefore
the one thing that cannot serve as a checkpoint. And when a redelivery notice
says to probe live state before redoing work, probe the **worktree** and the
peer repo's worktree, not just the log — the honest answer is often "the work
exists, and is one killed process from never having happened".

**To message another repo's agent — one criterion, then three questions** (relay
spec v2.34 § *TL;DR*). The criterion: **Channel 0 addresses a context; the relay
addresses a role.**

1. **Is the ask for a live session that exists right now — one already holding this
   context, able to answer or coordinate now?** → `ListAgents` + `SendMessage`
   (Channel 0): push-style replies, no queue, no claim. A session is not a role, so
   verify the addressee first: reply-to-whoever-contacted-you > session name matches
   the task > **one-line probe before the payload** > it's the relay's job. Two
   durabilities, and only one moves you off this channel: a durable **record** is the
   tracker's job either way (peer messages for velocity, issues for the record) —
   durable **delivery** is the relay's: fall back when the ask is for *"whoever next
   works repo X"* or must survive the receiver's absence. A send confirms **enqueue,
   not delivery** (`idle` in the listing can be a session blocked on a modal prompt) —
   re-send an unanswered ask that matters on a relay channel, and address by **bare
   name**, since a `[ref]` decays across restarts. A peer message is never a
   permission decision.
2. **Else: must "not yet handled" stay visible to you?** → a Gitea `agent-relay`
   issue in the recipient repo — the only channel where an unhandled ask is a state
   the sender can see.
3. **Else: the recipient's primary (leftmost) registry channel** —
   `~/_sync/dev/infra/tools/relay-send --to <repo> …` for a `nats` row, a file in
   *their* `agent-relay/inbox/` for a file-inbox row. On NATS one decision remains,
   and it is a **class, not a fourth channel**: `--class auto` (the default — a
   headless handler, and a failed message leaves the work queue with only
   `RELAY_AUDIT` remembering it; measured 2026-08-19 on an onboarding ask that died
   unseen) only for work safe to run with nobody present; `interactive` when the
   answer must come back to a person.

## Repo rules (learned 2026-08-21 — evidence in `docs/2026-08-21-journal-day-bucketing.md`
and `docs/2026-08-21-identity-surfaces-and-query-floor.md`)

Only the things that bind from now on. The story, the measurements and the other lessons
are in the two docs — morning (the wrong-day journal bug) and afternoon (the v0.21.0
release and the query-floor measurements).

- **A row in `messages` is not a conversation turn.** 91.1 % of `claude`-provider rows
  have `content IS NULL` — they are sidecar state records (`permission-mode`,
  `agent-color`, `worktree-state`, …). Never size, quote or reason from a raw row count
  without saying which you mean; a session that is "67,889 messages" is 5,997
  conversation items. It is also why roughly half of any journal backfill legitimately
  posts `skip`. Tracked as #41.
- **An installed script is a COPY — a green `main` says nothing about what is running.**
  `~/.local/bin/cchv-distill` is installed, not symlinked, and announces no version, so a
  fix can sit undeployed while every checkbox says otherwise. Verify the *installed* file
  (`cmp` against `git cat-file -p HEAD:<path>`) before believing any claim about distiller
  behaviour. The rule is about the **install boundary**: when one task turns out
  undeployed, re-check every task on the far side of it, not just that one. Tracked as #40.
- **When a change alters the granularity of a key, sweep every predicate that tests that
  key.** The journal's day fold moved session → day and left three predicates behind — the
  transcript window, the arrival timestamp and the dirty check — two of which reached
  production. Grep for the old key and re-derive each hit before shipping, rather than
  fixing them as they surface.
- **Before trusting a check, name the reading that would make it FAIL. If you cannot, it
  is not a check.** Three in one afternoon, each of which produced a confident wrong answer:
  `count(DISTINCT uuid)` to measure ingest duplication (returns 1.0 always — `convert.rs`
  fills a missing uuid with a **random v4** per re-parse, and `content_hash` is populated on
  **0** rows); `curl --max-time N` to time an endpoint (abandons the request but leaves the
  query running server-side — six piled up and starved each other, so the reading was of
  our own backlog); and `until [ "$x" != "null" ]` to wait for a field (a *failed read* is
  also `!= "null"`, so it announced a tick that had not happened). Same family as
  `--is-ancestor` collapsing exit 1 and 128.
- **Publish the number from the machine that will run it.** Migration `0009` measured
  0.93 ms locally against a table of prod's exact shape and 7.15 ms on prod — **7.7×**.
  A local stand-in predicts the order of magnitude and nothing finer; measuring the wrong
  machine is its own kind of estimate, even when you did measure.
- **A measurement taken inside a cycle needs its position in that cycle, or it is a
  measurement of the day.** `messages` is append-only, so VACUUM is insert-triggered
  (~1.5 M inserts) and visibility-map coverage over the hot range decays between runs: the
  share of a 7-day window needing `Heap Fetches` swings **0 % → ~60 %**. Record
  `n_ins_since_vacuum` and `relallvisible`/`relpages` with any index timing, and say which
  way the current position biases it. Related: a threshold is not a schedule —
  `last_autovacuum` being old is not a backlog when no trigger has been crossed.
- **The release ceremony's own guards are in § Release Process and they are load-bearing**
  — `gh run list --commit <tag-sha>` across all workflows (not just `server-release.yml`),
  and a publication proof that reads `rc` by `case` because `--is-ancestor` exits 1 for
  "no" and 128 for "that object does not exist". Both were added after each was wrong
  twice in one day.

## Project Overview

Claude Code History Viewer is a Tauri-based desktop application that allows users to browse and analyze conversation history from multiple AI coding assistants: Claude Code (`~/.claude`), Codex CLI (`~/.codex`), OpenCode (`~/.local/share/opencode/`), GitHub Copilot CLI (`~/.copilot/session-state/`), and VS Code Copilot Chat (`<UserData>/workspaceStorage/<hash>/chatSessions/`).

## Development Commands

This project uses `just` (a command runner). Install with `brew install just` or `cargo install just`.

### Recommended (using just)

```bash
just setup          # Install dependencies and configure build environment
just dev            # Run full Tauri app in development mode (hot reload)
just lint           # Run ESLint
just tauri-build    # Build production app (macOS universal binary, Linux native)
just test           # Run vitest in watch mode
just test-run       # Run tests once with verbose output
just sync-version   # Sync version from package.json to Cargo.toml, tauri.conf.json, cchv-distill.py
```

### Alternative (using pnpm directly)

```bash
pnpm install                                    # Install dependencies
pnpm exec tauri dev                             # Development mode
pnpm exec tauri build --target universal-apple-darwin  # macOS build
pnpm exec tauri build                           # Linux/Windows build
pnpm dev                                        # Start Vite dev server only
pnpm build                                      # Build frontend with TypeScript checking
pnpm lint                                       # Run ESLint
```

## Branch Strategy

This is a **hybrid fork** with two distinct flows:

- **Fork work (the default).** `main` is the integration *and* release line.
  Day-to-day changes (the archive stack, fixes, features) land on `main`
  directly, or via short-lived `feature/*` / second-loop worktree branches that
  merge back into `main`. Releases are cut from `main` as `cchv-v*` tags (see
  Version Management). There is **no fork `develop` gate** — commit to `main`
  with sufficient granularity.
- **Contributing back to upstream.** `jhlee0409/claude-code-history-viewer` uses
  a `feature/* → develop → main` flow, so an upstream PR branches from
  `upstream/develop` and targets it (e.g. `feature/pi-provider` → PR
  jhlee0409#445). The `develop` branch exists **only** for this; it is not the
  fork's integration branch.

Upstream is the fork's **parser supply chain**: each sync ports `jhlee0409`
parser fixes into `crates/history-core` on `main`.

## Agent skills

mattpocock 스킬(`/triage`, `/to-issues`, `/diagnose`, `/improve-codebase-architecture` 등)이 참조하는 메타 설정.

### Issue tracker

GitHub Issues at `jhlee0409/claude-code-history-viewer`. See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles. `needs-info` and `wontfix` reuse existing repo labels;
`needs-triage`, `ready-for-agent`, `ready-for-human` are added in this setup.
See `docs/agents/triage-labels.md`.

### Domain docs

Single-context — one `CONTEXT.md` + `docs/adr/` at the repo root, lazily created
by `/grill-with-docs`. See `docs/agents/domain.md`.

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
propagates it to the Rust workspace, the Tauri config, and the distiller:

```
package.json (version)
    ↓ just sync-version
├── Cargo.toml  [workspace.package] version   ← every crate inherits it
│                                               (version.workspace = true);
│                                               the hub reports it on /v1/healthz
├── src-tauri/tauri.conf.json
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

# ===== Backend 검증 =====
cd src-tauri && cargo test -- --test-threads=1 && cd ..  # Rust 테스트 (단일 스레드 필수)
cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings && cd ..  # Rust 린트
cd src-tauri && cargo fmt --all -- --check && cd ..      # Rust 포맷 체크

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
just sync-version                            # package.json → workspace + tauri.conf
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
# (#40, 2026-08-21: the distiller's DISTILL_VERSION is sync-version's fifth target).
# Anything else that belongs in the release gets committed deliberately BEFORE this
# point, never swept in here.
git add package.json Cargo.toml Cargo.lock src-tauri/tauri.conf.json scripts/cchv-distill.py

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

### Desktop app (retired as a *distribution*, not as a dependency)

The Tauri desktop **distribution** and its auto-updater are retired: we build no
`.dmg`/`.app`, and the desktop release workflows are gone (`updater-release.yml`,
`updater-release-retry.yml`).

**What "retired" does NOT mean — verify before you repeat it.** An earlier
version of this section claimed `src-tauri` "now builds solely as the WebUI
server (`--features webui-server`)". That is false, and believing it leads to
wrong conclusions about CI. The facts, checked 2026-07-26:

- `src-tauri/Cargo.toml` has `default = []`, but `tauri` **and 10
  `tauri-plugin-*` crates are unconditional dependencies** — not optional, not
  feature-gated. `webui-server` only *adds* axum/tower/rust-embed; it subtracts
  nothing.
- Consequently every compile of `src-tauri`, including `--features
  webui-server`, drags in the full webview stack — 38 unambiguously
  desktop-only entries in `Cargo.lock` (`tauri*`, `wry`, `tao`, `webkit2gtk*`,
  `gtk*`, `gdk*`, `atk*`, `javascriptcore*`, `soup*`). **This is why
  `rust-tests.yml` installs `libgtk-3-dev` + `libwebkit2gtk-4.1-dev` on every
  Ubuntu job** — it is forced by the dependency graph, not an oversight.
- The desktop GUI still runs. `src-tauri/src/lib.rs::run()` dispatches
  `--export` (line ~95) then `--serve` (line ~106), and otherwise falls through
  to `tauri::Builder::default()` (line ~152).

Making that sentence true is the job of the **web-only-cut** work (memory's
"Deliverable 2"; no `openspec/changes/web-only-cut/` exists yet) — make `tauri`
optional, or split the CLI + WebUI server out of `src-tauri` into its own crate.
Until then, do not trim the GTK/webkit steps out of CI expecting it to work.

The updater code is **dormant / vestigial** (safe to remove in a future
cleanup): `src-tauri/src/commands/update.rs`, `src/hooks/useGitHubUpdater.ts`,
`src/hooks/useSmartUpdater.ts`, the tauri updater plugin in
`src-tauri/tauri.conf.json`, and the whole `update-flow-tests.yml` workflow,
which guards only retired updater UI.

### What CI builds, and who consumes it

Checked 2026-07-26. Useful when deciding whether a red check is worth fixing.

| Artifact | Built by | Consumer |
|---|---|---|
| `cchv-webapp.tar.gz` | `server-release.yml`, every `cchv-v*` tag | **Us** — infra swaps it into m4m's `static_dir`; the live archive browser |
| `cchv-hub-<v>-aarch64-apple-darwin` + `.sha256` | `server-release.yml`, every tag (macos-14) | **Us** — becomes `~/.local/bin/cchv-hub` on m4m (`docs/archive/deployment.md` §2b) |
| 4× WebUI server binaries (`src-tauri --features webui-server`) | `server-release.yml`, **dispatch-only** | **Nobody today.** Free unless dispatched |
| Desktop bundles | — | Not built at all |

`src-tauri` therefore ships to no one — but it is **not dead code**. It is the
local CLI the `cchv-find` skill §3 drives, built from source on demand:
`--export <id|path> --format html` and `--serve` both verified working
2026-07-26. So `rust-tests.yml` guards a real local tool, just not a shipped
artifact — weigh it accordingly.

Test workflows by what they guard: `archive-tests.yml` → the crates we actually
ship (history-core, protocol, hub, sync-daemon). `frontend-tests.yml` → the
webapp we ship. `rust-tests.yml` → the local-only CLI. `update-flow-tests.yml`
→ a retired feature.

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

## Architecture

### Data Flow

```
Claude Code:        ~/.claude/projects/[project]/*.jsonl                              ─┐
Codex CLI:          ~/.codex/sessions/**/rollout-*.jsonl                               │
OpenCode:           ~/.local/share/opencode/storage/                                   │
Copilot CLI:        ~/.copilot/session-state/<id>/events.jsonl   (workspace.yaml:      ├→ Rust Backend → Tauri IPC → React Frontend → Virtual List
Copilot Desktop:    ~/.copilot/session-state/<id>/events.jsonl    client_name routes)  │
VS Code Copilot:    <UserData>/workspaceStorage/<hash>/chatSessions/*.jsonl            ─┘
```

### Frontend (React + TypeScript)

- **State Management**: Uses Zustand store in `src/store/useAppStore.ts`
- **Components**: Located in `src/components/`
  - `MessageViewer.tsx` - Displays messages with virtual scrolling for performance
  - `ProjectTree.tsx` - Shows project/session hierarchy
  - `contentRenderer.tsx` - Handles rendering of different content types
  - `messageRenderer.tsx` - Renders tool use, tool results, and message content
- **API Integration**: Frontend communicates with Rust backend via Tauri's IPC commands
- **Virtual Scrolling**: Uses react-window for efficient rendering of large message lists

### Backend (Rust + Tauri)

- **Main Commands** (in `src-tauri/src/lib.rs`):
  - `get_claude_folder_path` - Locates user's `.claude` directory
  - `scan_projects` - Scans for all Claude projects
  - `load_project_sessions` - Loads sessions for a specific project
  - `load_session_messages` - Loads messages from a JSONL file
  - `search_messages` - Searches across all messages
- **Data Structure**: Reads JSONL files containing conversation history from `~/.claude/projects/`

## i18n Structure (Internationalization)

### File Structure (Namespace 기반)

LLM이 파악하기 좋은 namespace 기반 구조로 분리됨 (각 파일 2-40KB):

```
src/i18n/
├── index.ts                  # i18n configuration (namespace 병합)
├── useAppTranslation.ts      # Type-safe custom hook
├── types.generated.ts        # Auto-generated types (DO NOT EDIT)
└── locales/
    ├── en/                   # English (1392 keys total)
    │   ├── common.json       # 공통 UI (~99 keys)
    │   ├── analytics.json    # 분석 대시보드 (~132 keys)
    │   ├── session.json      # 세션/프로젝트 (~116 keys)
    │   ├── settings.json     # 설정 관리자 (~501 keys)
    │   ├── tools.json        # 도구 관련 (~69 keys)
    │   ├── error.json        # 에러 메시지 (~37 keys)
    │   ├── message.json      # 메시지 뷰어 (~66 keys)
    │   ├── renderers.json    # 렌더러 컴포넌트 (~255 keys)
    │   ├── update.json       # 업데이트 관련 (~65 keys)
    │   ├── feedback.json     # 피드백 (~32 keys)
    │   └── recentEdits.json  # 최근 편집 (~20 keys)
    ├── ko/                   # Korean (동일 구조)
    ├── ja/                   # Japanese (동일 구조)
    ├── zh-CN/                # Simplified Chinese (동일 구조)
    └── zh-TW/                # Traditional Chinese (동일 구조)
```

### Namespace 구조의 장점

1. **LLM 친화적**: 각 namespace 파일이 단일 컨텍스트에서 처리 가능한 크기
2. **관심사 분리**: 특정 기능 수정 시 해당 namespace 파일만 변경
3. **병렬 번역 가능**: 여러 기능을 독립적으로 번역 작업 가능
4. **기존 호환성**: `t('prefix.key')` 형식 그대로 동작

### Key Structure (Flat with Dot Notation)

각 namespace 파일 내에서 dot notation 사용:

```json
// locales/en/common.json
{
  "common.appName": "Claude Code History Viewer",
  "common.loading": "Loading...",
  "common.cancel": "Cancel"
}

// locales/en/analytics.json
{
  "analytics.dashboard": "Analytics Dashboard",
  "analytics.tokenUsage": "Token Usage"
}
```

### Namespace → Prefix 매핑

| Namespace | 포함 Prefix | Keys |
|-----------|-------------|------|
| `common` | common, status, time, copyButton | ~99 |
| `analytics` | analytics | ~132 |
| `session` | session, project | ~116 |
| `settings` | settingsManager, settings, folderPicker | ~501 |
| `tools` | tools, toolResult, toolUseRenderer, collapsibleToolResult | ~69 |
| `error` | error | ~37 |
| `message` | message, messages, messageViewer, messageContentDisplay | ~66 |
| `renderers` | advancedTextDiff, agentProgressGroup, agentTaskGroup, assistantMessageDetails, bashCodeExecutionToolResultRenderer, captureMode, citationRenderer, claudeContentArrayRenderer, claudeSessionHistoryRenderer, claudeToolUseDisplay, codeExecutionToolResultRenderer, codebaseContextRenderer, commandOutputDisplay, commandRenderer, contentArray, diffViewer, fileContent, fileEditRenderer, fileHistorySnapshotRenderer, fileListRenderer, gitWorkflowRenderer, globalSearch, imageRenderer, mcpRenderer, progressRenderer, queueOperationRenderer, structuredPatch, summaryMessageRenderer, systemMessageRenderer, taskNotification, taskOperation, terminalStreamRenderer, textEditorCodeExecutionToolResultRenderer, thinkingRenderer, toolSearchToolResultRenderer, webFetchToolResultRenderer, webSearchRenderer | ~255 |
| `update` | updateModal, updateSettingsModal, simpleUpdateModal 등 | ~65 |
| `feedback` | feedback | ~32 |
| `recentEdits` | recentEdits | ~20 |

### Usage in Components

```typescript
import { useTranslation } from 'react-i18next';

const MyComponent = () => {
  const { t } = useTranslation();

  return (
    <div>
      <h1>{t('common.appName')}</h1>
      <p>{t('session.title')}</p>
      <button>{t('common.cancel')}</button>
    </div>
  );
};
```

### i18n Scripts

```bash
pnpm run generate:i18n-types  # Regenerate types after adding keys
pnpm run i18n:validate        # Validate keys across all languages
pnpm run i18n:sync            # Sync keys across all languages
```

### Adding New Messages

1. **해당 namespace의 모든 언어 파일에 키 추가**:
   ```json
   // locales/en/common.json
   { "common.newKey": "New feature text" }

   // locales/ko/common.json
   { "common.newKey": "새 기능 텍스트" }
   // ... repeat for ja, zh-CN, zh-TW
   ```

2. **타입 재생성**:
   ```bash
   pnpm run generate:i18n-types
   ```

3. **검증**:
   ```bash
   pnpm run i18n:validate
   ```

### Adding New Language

1. 새 언어 디렉토리 생성 및 en 디렉토리 복사: `cp -r locales/en locales/es`
2. 각 namespace 파일 번역
3. `src/i18n/index.ts`에 언어 추가 (모든 namespace import)

### Key Sync Verification

```bash
# 검증 스크립트 실행
node scripts/validate-i18n.mjs
```

## Raw Message Structure

The application reads `.jsonl` files where each line is a JSON object representing a single message. The core structure is as follows:

```json
{
  "uuid": "...",
  "parentUuid": "...",
  "sessionId": "...",
  "timestamp": "...",
  "type": "user" | "assistant" | "system" | "summary",
  "message": { ... },
  "toolUse": { ... },
  "toolUseResult": { ... },
  "isSidechain": false
}
```

### The `message` Field

The `message` field is a nested JSON object. Its structure varies depending on the message `type`.

**For `user` messages:**

```json
{
  "message": {
    "role": "user",
    "content": "..." // or ContentItem[]
  }
}
```

**For `assistant` messages:**

Assistant messages contain additional metadata within the `message` object:

```json
{
  "message": {
    "id": "msg_...",
    "role": "assistant",
    "model": "claude-opus-4-20250514",
    "content": [...],
    "stop_reason": "tool_use" | "end_turn" | null,
    "usage": {
      "input_tokens": 123,
      "output_tokens": 456,
      "cache_creation_input_tokens": 20238,
      "cache_read_input_tokens": 0,
      "service_tier": "standard"
    }
  }
}
```

- **`id`, `model`, `stop_reason`, `usage`**: These fields are typically present only in assistant messages.
- **`usage` object**: Contains detailed token counts, including cache-related metrics.

## Key Implementation Details

- The app expects Claude conversation data in `~/.claude/projects/[project-name]/*.jsonl`
- Each JSONL file represents a session with one JSON message per line
- Messages can contain tool use results and error information
- The UI is primarily in Korean.션, etc.)
- Virtual scrolling is implemented for performance with large message lists
- Pagination is used to load messages in batches (100 messages per page)
- Message tree structure is flattened for virtual scrolling while preserving parent-child relationships
- No test suite currently exists

### CLI flags

- `--serve [--port N] [--host H] [--dist D] [--token T | --no-auth]` — WebUI headless mode (requires `webui-server` feature build). Parsed in `src-tauri/src/lib.rs::run_server`.
- `--session <uuid|uuid-prefix>` — preload a specific session at GUI startup. UUID regex accepts 8-36 hex-or-dash chars. Parsed in `src-tauri/src/cli.rs::parse_session_hint`, delivered to the frontend via the `get_startup_session_hint` Tauri command, resolved in `src/lib/preloadSession.ts`. A race guard inside `preloadSessionFromCli` respects user navigation made mid-scan.
- `--export <session-id|/abs/path.jsonl> [--format html|json] [--output <file>]` — **headless** session export (no GUI/webview); writes to `--output` or stdout, then exits. Dispatched first in `src-tauri/src/lib.rs::run`. Session ids resolve under `~/.claude/projects` (id prefix accepted when unambiguous). HTML rendering lives in `src-tauri/src/export.rs`, a Rust port of `src/services/export/{contentExtractor,htmlExporter}.ts` (markdown via `comrak`); keep the two in sync when adding content types.
- **Shared argv helper**: `src-tauri/src/cli_args.rs::extract_flag_value` is the canonical `--flag=value` / `--flag value` parser used by both the desktop and `webui-server` code paths.

### Static archive webapp

`just archive-web-build` → `dist-archive/`: a backend-free static build of the hub Archive mode (`archive.html` + `src/archive-main.tsx` + `ConnectGate`, own config `vite.archive.config.ts` so the Tauri/WebUI `dist/` is untouched). Deployable to any static host, or served by the hub itself via `static_dir` in `hub.toml` / `HUB_STATIC_DIR` env (`crates/hub`, router fallback — `/v1/*` always wins). Hub connection (URL + read token) is entered on first visit and persisted in browser localStorage. Spec: `openspec/specs/static-archive-webapp/spec.md`, `openspec/specs/hub-static-hosting/spec.md`; deploy notes: `docs/archive/deployment.md`.

## Important Patterns

- Tauri commands are async and return `Result<T, String>`
- Frontend uses `@tauri-apps/api/core` for invoking backend commands
- All file paths must be absolute when passed to Rust commands
- The app uses Tailwind CSS with custom Claude brand colors defined in `tailwind.config.js`
- Message components are memoized for performance
- AutoSizer is used for responsive virtual scrolling
- Message height is dynamically calculated and cached for variable height scrolling

## Claude Directory Structure Analysis

### Directory Structure

```text
~/.claude/
├── projects/          # Contains project-specific conversation data
│   └── [project-name]/
│       └── *.jsonl    # JSONL files with conversation messages
├── ide/              # IDE-related data
├── statsig/          # Statistics/analytics data
└── todos/            # Todo list data
```

### JSONL Message Format

Each JSONL file contains one JSON object per line. The actual structure differs from what the frontend expects:

#### Raw Message Structure (in JSONL files)

This is the corrected structure based on analysis of the `.jsonl` files.

```json
{
  "uuid": "unique-message-id",
  "parentUuid": "uuid-of-parent-message",
  "sessionId": "session-uuid",
  "timestamp": "2025-06-26T11:45:51.979Z",
  "type": "user | assistant | system | summary",
  "isSidechain": false,
  "cwd": "/path/to/working/directory",
  "version": "1.0.35",
  "requestId": "request-id-from-assistant",
  "userType": "external",
  "message": {
    "role": "user | assistant",
    "content": "..." | [],
    "id": "msg_...",
    "model": "claude-opus-4-20250514",
    "stop_reason": "tool_use",
    "usage": { "input_tokens": 123, "output_tokens": 456 }
  },
  "toolUse": {},
  "toolUseResult": "..." | {}
}
```

**Note:** The fields `parentUuid`, `isSidechain`, `cwd`, `version`, `requestId`, `userType`, `toolUse`, `toolUseResult` are optional. The fields `id`, `model`, `stop_reason`, `usage` are specific to assistant messages and are also optional.

### Content Types

#### 1. User Message Content Types

**Simple String Content**

```json
{
  "type": "user",
  "message": {
    "role": "user",
    "content": "더 고도화할 부분은 없을까?"
  }
}
```

**Array Content with tool_result**

```json
{
  "type": "user",
  "message": {
    "role": "user",
    "content": [
      {
        "tool_use_id": "toolu_01VDVUHPae8mbcpER7tbbHvd",
        "type": "tool_result",
        "content": "file content here..."
      }
    ]
  }
}
```

**Array Content with text type**

```json
{
  "type": "user",
  "message": {
    "role": "user",
    "content": [
      {
        "type": "text",
        "text": "Please analyze this codebase..."
      }
    ]
  }
}
```

**Command Messages**

```json
{
  "type": "user",
  "message": {
    "role": "user",
    "content": "<command-message>init is analyzing your codebase…</command-message>\n<command-name>/init</command-name>"
  }
}
```

#### 2. Assistant Message Content Types

**Text Content**

```json
{
  "type": "assistant",
  "message": {
    "role": "assistant",
    "content": [
      {
        "type": "text",
        "text": "I'll help you fix these Rust compilation errors..."
      }
    ]
  }
}
```

**Tool Use Content**

```json
{
  "type": "assistant",
  "message": {
    "role": "assistant",
    "content": [
      {
        "type": "tool_use",
        "id": "toolu_01QUa384MpVwU4F8tuF8hg9T",
        "name": "TodoWrite",
        "input": {
          "todos": [...]
        }
      }
    ]
  }
}
```

**Thinking Content**

```json
{
  "type": "assistant",
  "message": {
    "role": "assistant",
    "content": [
      {
        "type": "thinking",
        "thinking": "사용자가 메시지 객체의 내용이 null이고...",
        "signature": "EpUICkYIBRgCKkCB6bsN5FuO+M1gLbr..."
      }
    ]
  }
}
```

#### 3. Tool Use Result Structures

**File Read Results**

```json
{
  "toolUseResult": {
    "type": "text",
    "file": {
      "filePath": "/Users/jack/client/ai-code-tracker/package.json",
      "content": "{\n  \"name\": \"ai-code-tracker\"...",
      "numLines": 59,
      "startLine": 1,
      "totalLines": 59
    }
  }
}
```

**Command Execution Results**

```json
{
  "toolUseResult": {
    "stdout": "> ai-code-tracker@0.6.0 lint\n> eslint src --fix",
    "stderr": "",
    "interrupted": false,
    "isImage": false
  }
}
```

**Error Results**

```json
{
  "message": {
    "content": [
      {
        "type": "tool_result",
        "content": "Error: The service was stopped\n    at ...",
        "is_error": true,
        "tool_use_id": "toolu_01PKwT3i8u1ryjWZpMBWmDjX"
      }
    ]
  }
}
```

**Todo List Results**

```json
{
  "toolUseResult": {
    "oldTodos": [...],
    "newTodos": [...]
  }
}
```

**Multi-Edit Results**

```json
{
  "toolUseResult": {
    "filePath": "/Users/jack/client/ai-code-tracker/src/extension.ts",
    "edits": [
      {
        "old_string": "...",
        "new_string": "...",
        "replace_all": false
      }
    ],
    "originalFileContents": "..."
  }
}
```

#### 4. Special Message Types

**Summary Messages**

```json
{
  "type": "summary",
  "summary": "AI Code Tracker: Comprehensive VS Code Extension Analysis",
  "leafUuid": "28f1d1f6-3485-48a6-9408-723624bc1e42"
}
```

### Message Metadata Fields

- `parentUuid`: Links to parent message in conversation tree
- `isSidechain`: Boolean indicating if this is a sidechain conversation
- `userType`: Usually "external" for user messages
- `cwd`: Current working directory when message was sent
- `sessionId`: Unique session identifier
- `version`: Claude client version
- `timestamp`: ISO 8601 timestamp
- `uuid`: Unique message identifier
- `requestId`: Present in assistant messages

### Content Rendering Status

Currently Supported:

- ✅ Text content (`type: "text"`) - with citations support
- ✅ Tool use (`type: "tool_use"`)
- ✅ Tool results (`type: "tool_result"`)
- ✅ Command messages (within text content)
- ✅ Thinking content (`type: "thinking"`)
- ✅ Redacted thinking (`type: "redacted_thinking"`) - encrypted by safety systems
- ✅ Image content (`type: "image"`) - base64 and URL sources
- ✅ Server tool use (`type: "server_tool_use"`) - e.g., web_search
- ✅ Web search results (`type: "web_search_tool_result"`)
- ✅ Document content (`type: "document"`) - PDF and plain text
- ✅ Search results (`type: "search_result"`)
- ✅ MCP tool use (`type: "mcp_tool_use"`) - Model Context Protocol tool calls
- ✅ MCP tool result (`type: "mcp_tool_result"`) - MCP tool execution results
- ✅ Citations - inline source references

2025 Beta Content Types:
- ✅ Web fetch result (`type: "web_fetch_tool_result"`) - Full page/PDF content retrieval (beta: web-fetch-2025-09-10)
- ✅ Code execution result (`type: "code_execution_tool_result"`) - Legacy Python execution (beta: code-execution-2025-08-25)
- ✅ Bash execution result (`type: "bash_code_execution_tool_result"`) - Bash command execution (beta: code-execution-2025-08-25)
- ✅ Text editor result (`type: "text_editor_code_execution_tool_result"`) - File operations (beta: code-execution-2025-08-25)
- ✅ Tool search result (`type: "tool_search_tool_result"`) - MCP tool discovery (beta: mcp-client-2025-11-20)

Message-level Metadata (2025):
- ✅ `costUSD` - API usage cost
- ✅ `durationMs` - Response latency

### Recent Updates

- **2025 Beta Content Types Support (January 2026)**:
  - Added 5 new beta content type renderers:
    - `WebFetchToolResultRenderer` - Web page/PDF content retrieval
    - `CodeExecutionToolResultRenderer` - Legacy Python code execution
    - `BashCodeExecutionToolResultRenderer` - Bash command execution
    - `TextEditorCodeExecutionToolResultRenderer` - File view/create/edit/delete operations
    - `ToolSearchToolResultRenderer` - MCP tool discovery results
  - Added shared `safeStringify` utility in `src/utils/jsonUtils.ts`
  - Memoized `ClaudeContentArrayRenderer` for performance
- **2025 Content Types Support (December 2025)**:
  - Added support for new content types from Claude API 2025 updates
  - Implemented `redacted_thinking`, `server_tool_use`, `web_search_tool_result`, `document`, `search_result` renderers
  - Added `CitationRenderer` for inline source references
  - Added `costUSD` and `durationMs` fields to message metadata
  - Enhanced `AssistantMessageDetails` to display cost and duration metrics
- **Data Structure & Type Correction (June 2025)**:
  - Performed a deep analysis of `.jsonl` log files in the `~/.claude` directory to verify the exact data structure.
  - Added a `Raw Message Structure` section to this document to accurately model the nested `message` object and include assistant-specific metadata (`id`, `model`, `stop_reason`, `usage`).
  - Updated the corresponding Rust structs in `src-tauri/src/commands.rs` and TypeScript interfaces in `src/types/index.ts` to align with the true data format, enhancing type safety and preventing data loss during parsing.
- **Virtual Scrolling Implementation**: Added react-window with VariableSizeList for efficient rendering of large message lists
- **Performance Optimizations**:
  - Messages are memoized to prevent unnecessary re-renders
  - Dynamic height calculation for variable content sizes
  - AutoSizer for responsive viewport handling
  - Infinite scroll with react-window-infinite-loader
- **Type System Updates**:
  - Fixed ContentItem[] type support in ClaudeMessage interface
  - Added proper TypeScript types for virtual scrolling components
  - Updated messageAdapter to use type-only imports

### Dependencies Added

- `react-window` - Virtual scrolling for performance
- `react-window-infinite-loader` - Infinite scroll support
- `react-virtualized-auto-sizer` - Responsive height calculation
- `@types/react-window` - TypeScript definitions
- `@types/react-window-infinite-loader` - TypeScript definitions

### Known Issues

- The frontend expects content at the root level, but it's actually nested under `message.content`
- Thinking content appears both as a separate type and as tags within text
- Image support is defined in the data structure but not implemented in the UI
- ESLint configuration uses deprecated .eslintignore (migrated to ignores in config)

## Code Quality Checklist (PR #78 리뷰 기반)

코드 작성 시 아래 항목을 반드시 준수한다. 이 체크리스트는 PR #78에서 반복 발견된 34건의 리뷰 이슈를 예방하기 위한 것이다.

### 보안
- 사용자 입력 ID를 파일 경로에 사용할 때 → `^[A-Za-z0-9_-]+$` 검증 필수
- 파일 쓰기 → temp 파일 + rename 패턴(원자적 쓰기)
- Rust에서 디렉토리 순회 시 symlink 차단

### 에러 처리
- 모든 `async/await` → try/catch + 사용자에게 보이는 피드백 (toast/alert). `console.error`만은 부족
- 다단계 저장 → 모든 파싱/검증을 먼저 완료한 후 적용
- 필수 매개변수(`projectPath` 등) → 함수 시작부에 가드 배치

### i18n
- 새 키 추가 → 5개 locale 파일(en, ko, ja, zh-CN, zh-TW) 모두 동시 업데이트
- JSON 중복 키 절대 금지 — `pnpm run i18n:validate`로 검증
- TSX 내 사용자에게 보이는 문자열 → 반드시 `t()` 래핑

### 접근성 (a11y)
- 아이콘 전용 버튼 → `aria-label` 필수
- Dialog → `DialogTitle` 또는 `aria-label` 필수
- `Label`-`Input` 쌍 → `htmlFor`/`id` 연결, ID는 `React.useId()`
- `TooltipTrigger` → 포커스 가능한 요소(`<button>`)로 감싸기

### React 상태 관리
- `setState` 직후 해당 상태를 읽지 말 것 → 값을 인자로 직접 전달하거나 `useEffect` 사용
- 커스텀 훅 내부에서 다른 커스텀 훅 호출 → 인스턴스 분리 문제 주의

### 크로스 플랫폼
- 경로 split → `split(/[\\/]/)` (Windows `\` 대응)
- Rust `fs::rename` → Windows에서 대상 존재 시 실패, `remove_file` 후 rename
- 홈 디렉토리 감지 → `C:\Users\` 패턴 포함

### 기타
- 유틸리티 함수 작성 전 → 기존 utils에 동일 기능 있는지 확인
- null 체크 → `!= null`(loose equality)로 null+undefined 동시 처리
- `localStorage` 접근 → 항상 try/catch
