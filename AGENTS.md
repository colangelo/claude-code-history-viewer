# AGENTS.md

The guidance index for this repo — house conventions for agents *and*
project/code guidance, merged 2026-07-26 and split on 2026-10-30 into this index
plus `docs/agents/` (each moved section keeps its heading here, with a link to
the file that holds it word for word). **`CLAUDE.md` is a symlink to this
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

This rule exists because the *Desktop app (retired…)* section (`docs/agents/release.md`) asserted, until
2026-07-26, that `src-tauri` "now builds solely as the WebUI server". It does not,
and the false sentence propagated into reasoning about CI. Same for a red check:
know what a workflow guards before spending effort turning it green — see *What CI
builds, and who consumes it* (`docs/agents/release.md`) for the current artifact/consumer table.

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

Moved to [`docs/agents/agent-relay.md`](docs/agents/agent-relay.md): the `nats` relay participation, how to message another repo's agent (Channel 0, Gitea issue, `relay-send`; `auto` vs `interactive`). Two rules stay binding here: **run `/check-relay` first at every session start, after `/clear` and after a resume**, and **checkpoint by committing** (uncommitted work in this Syncthing-shared tree is unwritten).

## Repo rules

Moved to [`docs/agents/repo-rules.md`](docs/agents/repo-rules.md): the rules learned 2026-08-21 and after: a `messages` row is not a turn; an installed script is a copy; sweep every predicate when a key's granularity changes; name the reading that would make a check fail; publish the number from the machine that runs it; position in a cycle; poller phase; say who took a reading; ask which query produced a correction; a parameter with more than one effect; the release guards.

## Project Overview

Claude Code History Viewer is a web application — a React frontend served by a Rust WebUI server (`crates/viewer`, `--serve`), plus the cross-machine archive stack (`crates/hub`, `crates/sync-daemon`) — that lets users browse and analyze conversation history from multiple AI coding assistants: Claude Code (`~/.claude`), Codex CLI (`~/.codex`), OpenCode (`~/.local/share/opencode/`), GitHub Copilot CLI (`~/.copilot/session-state/`), and VS Code Copilot Chat (`<UserData>/workspaceStorage/<hash>/chatSessions/`).

## Development Commands

This project uses `just` (a command runner). Install with `brew install just` or `cargo install just`.

### Recommended (using just)

```bash
just setup          # Install dependencies and configure build environment
just dev            # Build the frontend and run the WebUI server on :3727 (serve-dev)
just lint           # Run ESLint
just serve-build    # Release WebUI server binary with the frontend embedded
just test           # Run vitest in watch mode
just test-run       # Run tests once with verbose output
just sync-version   # Sync version from package.json to Cargo.toml, cchv-distill.py
```

### Alternative (using pnpm directly)

```bash
pnpm install                                    # Install dependencies
pnpm dev                                        # Vite dev server only (VITE_MOCK=1 for a mock API)
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

Moved to [`docs/agents/release.md`](docs/agents/release.md): the `cchv-vX.Y.Z` line, `package.json` as the version source, `just sync-version`, the release ceremony (§ Release Process: quality gate, bump/tag/push with the conflict censuses, both Cargo.lock guards and the both-remotes publication proof, then CI and deploy), the removed desktop app, and § What CI builds, and who consumes it. Invoke the `cchv-deploy` skill before any release.

## Architecture

Moved to [`docs/agents/architecture.md`](docs/agents/architecture.md): data flow, frontend and backend structure, § Key Implementation Details, § CLI flags (`--serve`, `--export`), § Static archive webapp, § Important Patterns.

## i18n Structure (Internationalization)

Moved to [`docs/agents/i18n.md`](docs/agents/i18n.md): namespace layout, key structure, the namespace → prefix table, scripts (`pnpm run i18n:validate`), adding keys and languages.

## Raw Message Structure

Moved to [`docs/agents/data-format.md`](docs/agents/data-format.md): the JSONL transcript format (§ Raw Message Structure, § Claude Directory Structure Analysis: content types, tool results, metadata fields, rendering status, recent updates, known issues).

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
