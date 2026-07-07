# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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
and per-platform WebUI server binaries.

The always-on **m4m hub is NOT deployed from that release**. The real path is:
build locally → stage in `~/.config/cchv/staging/` → relay home-network (infra)
→ binary swap per `docs/archive/deployment.md`. The GitHub Release is for
reproducibility and non-local hosts.

```bash
gh run list --workflow=server-release.yml --limit=1
gh release view cchv-v0.6.0
```

#### Troubleshooting

| Problem | Cause | Fix |
|---------|-------|-----|
| CI pnpm version clash | `pnpm/action-setup` `version` vs `package.json` `packageManager` | drop `version` in the workflow (auto-detected) |
| `cargo test` flaky | `env::set_var("HOME")` is process-global → parallel race | `--test-threads=1` |
| Duplicate release | manual `gh release create` + workflow auto-create | let `server-release.yml` own it |
| Modules not found after `pnpm install` | lockfile ↔ node_modules drift | `rm -rf node_modules && pnpm install` |

### Desktop app (retired)

The Tauri desktop distribution and its auto-updater are **retired** — the fork
ships only the web viewer, so `src-tauri` now builds solely as the WebUI server
(`--features webui-server`). The desktop release workflows were removed
(`updater-release.yml`, `updater-release-retry.yml`). The updater code still
exists but is **dormant / vestigial** (safe to remove in a future cleanup):
`src-tauri/src/commands/update.rs`, `src/hooks/useGitHubUpdater.ts`,
`src/hooks/useSmartUpdater.ts`, and the tauri updater plugin in
`src-tauri/tauri.conf.json`.

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
