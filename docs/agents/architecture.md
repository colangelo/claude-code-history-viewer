# Architecture

> Moved word for word from `AGENTS.md` on 2026-10-30 (ac's yes, asks row a1030-39), which links here.

## Architecture

### Data Flow

```
Claude Code:        ~/.claude/projects/[project]/*.jsonl                              ─┐
Codex CLI:          ~/.codex/sessions/**/rollout-*.jsonl                               │
OpenCode:           ~/.local/share/opencode/storage/                                   │
Copilot CLI:        ~/.copilot/session-state/<id>/events.jsonl   (workspace.yaml:      ├→ Rust WebUI server → HTTP /api/* → React Frontend → Virtual List
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
- **API Integration**: `src/services/api.ts` POSTs JSON to the WebUI server's `/api/<command>` routes
- **Virtual Scrolling**: Uses react-window for efficient rendering of large message lists

### Backend (Rust WebUI server)

- **Main Commands** (in `crates/viewer/src/commands/`, routed by `crates/viewer/src/server/mod.rs`):
  - `get_claude_folder_path` - Locates user's `.claude` directory
  - `scan_projects` - Scans for all Claude projects
  - `load_project_sessions` - Loads sessions for a specific project
  - `load_session_messages` - Loads messages from a JSONL file
  - `search_messages` - Searches across all messages
- **Data Structure**: Reads JSONL files containing conversation history from `~/.claude/projects/`
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

- `--serve [--port N] [--host H] [--dist D] [--token T | --no-auth]` — WebUI headless mode (requires `webui-server` feature build). Parsed in `crates/viewer/src/lib.rs::run_server`.
- `--export <session-id|/abs/path.jsonl> [--format html|json] [--output <file>]` — headless session export; writes to `--output` or stdout, then exits. Dispatched first in `crates/viewer/src/lib.rs::run`. Session ids resolve under `~/.claude/projects` (id prefix accepted when unambiguous). HTML rendering lives in `crates/viewer/src/export.rs`, a Rust port of `src/services/export/{contentExtractor,htmlExporter}.ts` (markdown via `comrak`); keep the two in sync when adding content types.
- **Shared argv helper**: `crates/viewer/src/cli_args.rs::extract_flag_value` is the canonical `--flag=value` / `--flag value` parser used by both the `--export` and `--serve` code paths.

### Static archive webapp

`just archive-web-build` → `dist-archive/`: a backend-free static build of the hub Archive mode (`archive.html` + `src/archive-main.tsx` + `ConnectGate`, own config `vite.archive.config.ts` so the WebUI `dist/` is untouched). Deployable to any static host, or served by the hub itself via `static_dir` in `hub.toml` / `HUB_STATIC_DIR` env (`crates/hub`, router fallback — `/v1/*` always wins). Hub connection (URL + read token) is entered on first visit and persisted in browser localStorage. Spec: `openspec/specs/static-archive-webapp/spec.md`, `openspec/specs/hub-static-hosting/spec.md`; deploy notes: `docs/archive/deployment.md`.

## Important Patterns

- Backend commands are async and return `Result<T, String>`; a new one needs a handler and a route in `crates/viewer/src/server/`
- Frontend calls backend commands through `api()` in `src/services/api.ts`
- All file paths must be absolute when passed to Rust commands
- The app uses Tailwind CSS with custom Claude brand colors defined in `tailwind.config.js`
- Message components are memoized for performance
- AutoSizer is used for responsive virtual scrolling
- Message height is dynamically calculated and cached for variable height scrolling
