<div align="center">

<img src="docs/assets/app-icon.png" alt="CCHV Logo" width="120" />

# Claude Code History Viewer

**A self-hosted, cross-machine archive of your AI coding-agent history.**

A per-machine sync daemon pushes **Claude Code**, **Codex CLI**, **Copilot**, **Pi**, **OpenCode**, **Gemini CLI** and 20 more into a central Rust/Postgres hub with a search API and a web UI. Your history survives each tool's local retention window, and it's searchable from anywhere on your tailnet.

[![Release](https://img.shields.io/github/v/release/colangelo/claude-code-history-viewer?label=Release&color=blue)](https://github.com/colangelo/claude-code-history-viewer/releases)
[![License](https://img.shields.io/github/license/colangelo/claude-code-history-viewer)](LICENSE)
[![Archive Tests](https://img.shields.io/github/actions/workflow/status/colangelo/claude-code-history-viewer/archive-tests.yml?branch=main&label=Archive%20Tests)](https://github.com/colangelo/claude-code-history-viewer/actions/workflows/archive-tests.yml)
[![Frontend Tests](https://img.shields.io/github/actions/workflow/status/colangelo/claude-code-history-viewer/frontend-tests.yml?branch=main&label=Frontend%20Tests)](https://github.com/colangelo/claude-code-history-viewer/actions/workflows/frontend-tests.yml)
[![Last Commit](https://img.shields.io/github/last-commit/colangelo/claude-code-history-viewer)](https://github.com/colangelo/claude-code-history-viewer/commits/main)

[Deployment guide](docs/archive/deployment.md) · [Releases](https://github.com/colangelo/claude-code-history-viewer/releases) · [Upstream project](https://github.com/jhlee0409/claude-code-history-viewer)

</div>

---

> **This is a hard fork, and it is server-first.**
> [`jhlee0409/claude-code-history-viewer`](https://github.com/jhlee0409/claude-code-history-viewer)
> is an excellent cross-provider **desktop** viewer. This fork keeps its parsers and
> its UI, but re-points the product at a different problem: a durable, *central*,
> multi-machine archive. **We ship no desktop app** — no `.dmg`, no `.exe`, no
> `.AppImage`, no auto-updater, no Homebrew cask. What we ship is the archive stack
> and a static web UI. See [Relationship to upstream](#relationship-to-upstream).

## Why

AI coding agents delete local history on a fixed window — Claude Code keeps roughly
30 days — and every tool keeps its own history in its own format, on whichever
machine you happened to be sitting at. So the conversation where you worked out
*why* the migration had to run in two passes is gone, or it's on the other laptop.

This fork fixes that with three properties:

- **Cumulative.** The daemon only ever ingests. Deleting a local file never deletes
  anything from the hub. Once a message lands, it stays.
- **Cross-machine.** Every machine pushes to one hub. One search covers all of them.
- **Cross-provider.** 26 agents, one normalized schema, one search box.

## Architecture

```
each machine:  sync-daemon ──(HTTPS over Tailscale, bearer token)──▶  hub ──▶ Postgres
                  │                                                    │
                  └─ reads ~/.claude, ~/.codex, ~/.pi, …               └─ /v1/ingest /v1/search
                     via the shared history-core parser                   /v1/projects /v1/journal …
                                                                          (the ONLY component
                                                                           with DB credentials)
                                                                                 │
                                                       static archive webapp ────┘
                                                       (served by the hub at /)
```

| Crate | Role |
|-------|------|
| [`crates/history-core`](crates/history-core) | Tauri-free extraction and normalization of every provider's history. The shared parser — the daemon, the hub and the local viewer all use it. |
| [`crates/sync-daemon`](crates/sync-daemon) | Per-machine daemon. Backfills, then incrementally pushes local history to the hub. Crash-safe checkpoints; holds only a hub URL + token. |
| [`crates/hub`](crates/hub) | axum + sqlx service. The only component with DB credentials. Bearer-authed `/v1/ingest`, plus the read API: full-text + semantic search, browse, journal, analytics. |
| [`crates/protocol`](crates/protocol) | The wire types shared by daemon and hub. |
| `dist-archive/` | The static archive webapp (`just archive-web-build`) — backend-free, served by the hub itself or any static host. |

Transport security is Tailscale (WireGuard); a bearer token gates the data
endpoints. Read access can additionally be gated on Tailscale identity
(`trust_tailscale_identity`).

## Quick Start (self-hosting)

Full instructions — including systemd/launchd units, TLS, secrets, and the
homelab-specific bits — are in **[`docs/archive/deployment.md`](docs/archive/deployment.md)**.
The short version:

**1. Postgres.** 12 or newer (the schema uses generated stored columns). The hub
applies its own migrations on startup, so there is no manual migration step, and
no extensions are required.

```bash
createdb cchv_archive
psql -d cchv_archive -c "CREATE ROLE cchv LOGIN PASSWORD 'CHANGE_ME';"
psql -d cchv_archive -c "GRANT ALL ON DATABASE cchv_archive TO cchv;"
```

**2. Hub**, on your always-on node. Either build it (`cargo build --release -p hub`)
or grab `cchv-hub-<version>-aarch64-apple-darwin` from
[Releases](https://github.com/colangelo/claude-code-history-viewer/releases).

```toml
# /etc/cchv/hub.toml — one [[tokens]] entry per machine
database_url = "postgres://cchv:CHANGE_ME@localhost/cchv_archive"
bind_addr    = "0.0.0.0:8787"          # reachable over the tailnet
static_dir   = "/path/to/dist-archive" # optional: serve the web UI at /

[[tokens]]
token      = "GENERATE_A_LONG_RANDOM_SECRET"
machine_id = "11111111-1111-1111-1111-111111111111"
label      = "laptop"
```

```bash
HUB_CONFIG=/etc/cchv/hub.toml ./target/release/hub
curl http://<host>:8787/v1/healthz     # {"status":"ok","db":"up"}
```

**3. Sync daemon**, on every machine you code from
(`cargo build --release -p sync-daemon`):

```toml
# ~/.config/cchv/daemon.toml
hub_url            = "http://<tailnet-host>:8787"
hub_token          = "GENERATE_A_LONG_RANDOM_SECRET"   # this machine's token
scan_interval_secs = 3600
```

The daemon persists a stable machine id at `~/.claude-history-sync/machine_id` on
first run and prints it — put that id in the hub's `hub.toml` alongside this
machine's token.

**4. Web UI.** `just archive-web-build` produces `dist-archive/`. Point the hub's
`static_dir` (or `HUB_STATIC_DIR`) at it and the hub serves the UI and the API from
one origin — no CORS, no mixed content. `/v1/*` always wins over static files;
static assets are served without auth, the bearer token still gates all data.

The webapp is also published as `cchv-webapp.tar.gz` on every release, if you'd
rather not build it.

## What the archive gives you

| Capability | What it does |
|---|---|
| **Journal** | Day-grouped distilled summaries of what each session was actually about, so you can skim weeks at a glance instead of scrolling transcripts. Deep-linkable. |
| **Search** | Postgres full-text with prefix matching, **plus** hub-local semantic embeddings — `mode=hybrid` fuses both and degrades to keyword if the embedder is unavailable. Results land you on the exact message, in context. |
| **Browse** | Projects → sessions → messages, with provider badges, role/timestamp gutters, and worktree/clone grouping. |
| **Project identity** | Moved, cloned and worktree copies of the same repo are grouped by git fingerprint rather than by path, so a `~/dev/foo` → `~/work/foo` move doesn't fork your history. |
| **Analytics** | Token and cost breakdowns across every machine and provider, not just the one you're sitting at. |
| **Time Machine backfill** | `just tm-backfill` recovers history older than the local retention window from Time Machine backups — including from retired machines, via their backup disk. See [`docs/archive/timemachine-backfill.md`](docs/archive/timemachine-backfill.md). |

## Supported providers

**Twenty-six assistants, one archive.** Every provider below is parsed by
`history-core`, so it is browsable locally *and* ingested into the hub.

| Provider | Data Location | What You Get |
|----------|--------------|--------------|
| **Claude Code** | `~/.claude/projects/` | Full conversation history, tool use, thinking, costs |
| **GitHub Copilot** | `~/.copilot/session-state/` (CLI & Desktop), VS Code `workspaceStorage/.../chatSessions/` | Copilot CLI, Copilot Desktop, and VS Code Copilot Chat history (read-only, WSL-aware) |
| **Codex CLI** | `~/.codex/sessions/` | Session rollouts with agent responses |
| **Pi** | `~/.pi/agent/sessions/` | badlogic's `pi` / Pi SDK agent — JSONL transcripts per working directory |
| **OpenCode** | `~/.local/share/opencode/` | Conversation sessions and tool results |
| **Gemini CLI** | `~/.gemini/history/` | Conversation history with tool calls |
| **Antigravity** | `~/.gemini/antigravity/` | Conversation state under `brain/` plus token monitor data under `.token-monitor/rpc-cache/v1/` |
| **Cline** (incl. Roo Code, Kilo Code) | VS Code `globalStorage/<ext>/tasks/` | Task-based history across the Cline family |
| **Cursor** | `~/.cursor/` | Composer and chat conversations |
| **Cursor Agent** | `~/.cursor/projects/.../agent-transcripts/` | Agent transcripts, distinct from the Cursor IDE source |
| **Aider** | Project directories | Chat history and edit logs |
| **ForgeCode** | `~/.forge/.forge.db` | Conversation history from SQLite database |
| **CodeBuddy Code** | `~/.codebuddy/projects/` | Conversation history with tool calls (Claude Code fork format) |
| **Kimi** | `~/.kimi/` | Session history with `kimi -r` resume |
| **Kiro** | `kiro-cli/data.sqlite3` | SQLite-backed conversation history |
| **Amazon Q CLI** | `…/amazon-q/data.sqlite3` | SQLite `conversations` store (shares format with the Kiro CLI provider) |
| **Continue.dev** | `~/.continue/sessions/*.json` | Per-session JSON, grouped by workspace (honors `CONTINUE_GLOBAL_DIR`) |
| **PearAI** | `~/.pearai/sessions/` | Continue fork — same session format |
| **Goose** | `…/goose/sessions/sessions.db` | Block's agent — SQLite sessions + messages |
| **Crush** | per-project `./.crush/crush.db` | Charm's TUI — SQLite, discovered across common code roots |
| **llm** | `…/io.datasette.llm/logs.db` | Simon Willison's CLI — SQLite conversations/responses with token counts |
| **Open Interpreter** | `~/.openinterpreter/sessions/` | Codex-format rollouts (reuses the Codex parser; `INTERPRETER_HOME` override) |
| **Qwen Code** | `~/.qwen/projects/.../chats/` | Per-session JSONL transcripts (tool calls, thinking, token usage) |
| **Zed** | `…/Zed/threads/threads.db` | Agent Panel threads — SQLite + Zstd-compressed JSON |
| **OpenHands** | `~/.openhands/sessions/` | Classic event-store conversations |
| **Trae** | `…/Trae/User/workspaceStorage/.../state.vscdb` | Per-workspace chat (icube store; experimental, reverse-engineered) |

Adding a provider means adding one module under
[`crates/history-core/src/providers/`](crates/history-core/src/providers) and
registering it in `mod.rs` — the daemon, hub and viewer pick it up for free.

## Local viewer and CLI

`src-tauri` is no longer a shipped artifact, but it is not dead code: it's the
local tool you build from source when you want to read *this* machine's history
without a hub, or export a session.

<p align="center">
  <img width="49%" alt="Conversation History" src="https://github.com/user-attachments/assets/9a18304d-3f08-4563-a0e6-dd6e6dfd227e" />
  <img width="49%" alt="Analytics Dashboard" src="https://github.com/user-attachments/assets/0f869344-4a7c-4f1f-9de3-701af10fc255" />
</p>

```bash
# Headless HTTP server — browse this machine's local history in a browser
just serve-build-run                  # → http://localhost:3727 (token printed to stderr)

# Headless export — no GUI, no webview; writes and exits
cargo run -p claude-code-history-viewer -- \
  --export <session-id|/abs/path.jsonl> --format html --output report.html
```

| Flag | Description |
|------|-------------|
| `--serve` | Start the HTTP server instead of the GUI. Takes `--port`, `--host`, `--dist`, `--token`, `--no-auth`. Requires a `--features webui-server` build. |
| `--export <id\|path>` | Render one session to HTML or JSON (`--format`, `--output`) and exit. Session ids resolve under `~/.claude/projects`; an unambiguous prefix is accepted. |
| `--session <uuid\|prefix>` | Launch the GUI pre-focused on a session. |

Server-mode details (Docker, systemd, reverse proxies, auth) are in the
[Server Mode Guide](docs/server-guide.md) ([한국어](docs/server-guide.ko.md)).

## Build from source

```bash
git clone https://github.com/colangelo/claude-code-history-viewer.git
cd claude-code-history-viewer

just setup                # install deps, configure the build environment
just archive-web-build    # static archive webapp → dist-archive/
cargo build --release -p hub -p sync-daemon
```

**Requirements:** Node.js 18+, pnpm, Rust 1.80+ (the hub's embedder dependency
graph sets the floor; the other crates are fine at 1.77.2). Building `src-tauri`
additionally needs the platform webview toolchain — on Debian/Ubuntu,
`libgtk-3-dev` and `libwebkit2gtk-4.1-dev`.

Common recipes (`just --list` for the rest):

| Recipe | What it does |
|--------|--------------|
| `just archive-web-build` | Static archive webapp → `dist-archive/` |
| `just serve-build-run` | Build + run the local WebUI server |
| `just rust-check-all` | `fmt --check` + clippy + tests |
| `just test-run` | Frontend tests, once, verbose |
| `just lint` | ESLint |
| `just sync-version` | Propagate `package.json` version → Cargo workspace + Tauri config |
| `just tm-backfill` | Recover old history from Time Machine backups |

Tests run single-threaded on the Rust side (`cargo test -- --test-threads=1`) —
the settings tests set `HOME` process-globally.

## Data privacy

**Self-hosted, no third parties.** Nothing leaves the infrastructure you run. The
daemon talks only to the hub you configured; the hub talks only to your Postgres.
No analytics, no tracking, no telemetry, no cloud service.

The archive is, by design, a *durable* copy of conversations that would otherwise
expire — treat the hub and its database as sensitive, and put them behind a
private network.

## Relationship to upstream

Upstream is this fork's **parser supply chain**. Each sync ports
[`jhlee0409`](https://github.com/jhlee0409/claude-code-history-viewer)'s provider
fixes and new providers into `crates/history-core`, and fixes that belong upstream
go back as PRs against its `develop` branch.

The two projects have diverged in product, not in parsing:

| | Upstream (`jhlee0409`) | This fork |
|---|---|---|
| **Shape** | Desktop app (macOS/Windows/Linux) + optional headless server | Archive stack: daemon → hub → Postgres → static web UI |
| **Scope** | One machine's local files | Every machine, centrally, cumulatively |
| **Distribution** | `.dmg` / `.exe` / `.AppImage`, Homebrew cask, auto-updater | Hub binary + `cchv-webapp.tar.gz` on each release; build the rest from source |
| **Versions** | `v1.x` | `cchv-vX.Y.Z` (`0.x` — pre-stable, dogfood tier) |

Upstream's `v1.x` tags are fetched for the parser supply chain but are not ours;
[`CHANGELOG.md`](./CHANGELOG.md) is upstream's release history. For this fork's
history, see `git tag -n 'cchv-v*'` and the
[Releases](https://github.com/colangelo/claude-code-history-viewer/releases) page.

The desktop *distribution* is retired here; the desktop *dependency* is not.
`src-tauri` still compiles the full webview stack and its GUI path still runs —
see `AGENTS.md` for exactly what that does and does not mean before you act on it.

## Contributing

This fork is primarily a personal, dogfooded archive, so the useful contribution
paths are:

- **Provider parsers and parser fixes → upstream.** They benefit everyone, and
  they flow back here on the next sync.
- **Archive stack (hub, daemon, protocol, webapp) → here.** Non-trivial changes
  are specced through OpenSpec first (`openspec/changes/<name>/`) — see
  `AGENTS.md`.

Before committing:

```bash
pnpm tsc --build . && pnpm vitest run && pnpm lint && pnpm run i18n:validate
just rust-check-all
```

## License

[MIT](LICENSE) — free for personal and commercial use. Copyright for the original
work remains with JaeHyeok Lee; fork changes are MIT under the same terms.
