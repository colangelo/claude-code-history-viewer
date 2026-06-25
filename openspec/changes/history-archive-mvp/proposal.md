## Why

Claude Code (and several other agents) delete local conversation history after a fixed window — Claude Code's is 30 days — and every machine's history is siloed on that machine's disk. There is no durable, searchable, cross-machine record of past coding sessions. This project already normalizes ~16 agents into one schema and proves its parsers run headlessly; we can reuse that to build a personal archive that never forgets and is searchable from any machine. This is the MVP (durable archive + search API); semantic search (pgvector) and agent-context injection (MCP) are deliberately deferred to later phases but the schema is designed to accommodate them.

## What Changes

- Restructure the (forked) repo into a **Cargo workspace** and extract the existing tauri-coupled parsers into a new **tauri-free `history-core` library crate**. The desktop app keeps building and behaving exactly as before — it depends on `history-core` instead of owning the parsers. **No user-visible behavior change to the desktop app.**
- Add a **per-machine `sync-daemon`** binary that backfills all local agent history into a central archive, then keeps it current incrementally (file-watch + periodic safety-net rescan), with crash-safe local checkpointing and **cumulative archive semantics** (local deletions never delete archived rows).
- Add a central **`hub` service** (axum + sqlx) — the only component that touches Postgres — exposing a bearer-authenticated **`/ingest`** endpoint with idempotent upserts and a Postgres-backed schema (normalized columns + raw-fidelity JSONB + full-text search), designed pgvector-ready.
- Add **search and browse query endpoints** to the hub (`/search` via Postgres FTS, plus `/projects`, `/sessions`, `/sessions/:id/messages`) reachable from any machine over the Tailscale tailnet, mirroring the existing webui-server endpoint shapes.
- Add **sqlx migrations** (`migrations/`) and the deployment story for self-hosting Postgres + hub on one always-on tailnet node.

Out of scope for this change (designed-for, not built): pgvector embeddings + semantic search, an MCP agent-context server, pointing the React viewer at the hub, cross-machine logical project merging by git remote, and multi-user/cloud deployment.

## Capabilities

### New Capabilities
- `history-core-library`: A tauri-free Rust library crate that owns provider detection and the parse/normalize pipeline for all supported agents, exposing a stable headless API (`detect`, `scan_projects`, `load_sessions`, `load_messages`) over the normalized `ClaudeMessage`/`ClaudeSession`/`ClaudeProject` models, consumed by both the desktop app and the sync daemon.
- `history-sync-daemon`: A per-machine background binary that performs an initial full backfill of local agent history and then incremental synchronization (append-offset tracking for JSONL, re-diff for rewritten/SQLite formats, debounced file-watching, periodic safety-net rescan), batching records to the hub with at-least-once delivery, crash-safe checkpointing, stable machine identity, and cumulative (never-delete) semantics.
- `archive-ingestion`: The hub's ingest endpoint and Postgres persistence layer — a bearer-authenticated batched `/ingest` API and the schema (machines, projects, sessions, messages with normalized + raw-fidelity + FTS columns) that performs idempotent upserts and maintains session aggregates, designed to accommodate pgvector later.
- `archive-search-api`: The hub's read API reachable from any machine — full-text search over archived messages (ranked, filterable by provider/machine/project/time) plus browse/query endpoints for projects, sessions, and a session's messages.

### Modified Capabilities
<!-- None. No existing OpenSpec specs in this repo; the desktop app behavior is preserved, not modified at the requirement level. -->

## Impact

- **New crates**: `crates/history-core` (lib), `crates/sync-daemon` (bin), `crates/hub` (bin). Repo root gains a `[workspace]` `Cargo.toml` and a `migrations/` directory.
- **Refactor**: `src-tauri/src/{models,providers,utils.rs}` (+ GUI-independent parse logic in `export.rs`) relocate into `history-core`; `src-tauri` depends on it and keeps its `#[tauri::command]` wrappers. Must keep the desktop app green (`cargo test --test-threads=1`, `clippy -D warnings`, `cargo fmt --check`).
- **New dependencies**: `axum`, `sqlx` (Postgres, offline mode), `tokio`, an HTTP client (`reqwest`) for the daemon, `notify` for file-watching. `history-core` must NOT depend on `tauri`.
- **New infrastructure**: a self-hosted Postgres instance + the hub service on an always-on Tailscale node; per-machine daemon install (launchd/systemd) holding only a hub URL + bearer token.
- **CI**: add a throwaway Postgres service for hub integration tests and `sqlx` offline metadata; existing frontend (pnpm) and desktop validation unchanged.
