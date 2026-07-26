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
just sync-version   # Sync version from package.json to Cargo.toml and tauri.conf.json
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
propagates it to the Rust workspace and Tauri config:

```
package.json (version)
    ↓ just sync-version
├── Cargo.toml  [workspace.package] version   ← every crate inherits it
│                                               (version.workspace = true)
└── src-tauri/tauri.conf.json
```

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
pnpm tsc --build . && pnpm vitest run        # re-check after sync
git add -A && git commit -m "chore(release): cchv-v0.6.0"
git tag -a cchv-v0.6.0 -m "cchv-v0.6.0"
git push internal main && git push internal cchv-v0.6.0
git push origin  main && git push origin  cchv-v0.6.0
```

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
| `cchv-hub-<v>-aarch64-apple-darwin` + `.sha256` | `server-release.yml`, every tag (macos-14) | **Us** — becomes `/usr/local/bin/cchv-hub` on m4m (`docs/archive/deployment.md` §2b) |
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
