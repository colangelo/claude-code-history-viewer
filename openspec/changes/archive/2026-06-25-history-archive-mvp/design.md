## Context

This repository is a Tauri desktop app that already normalizes ~16 AI coding agents (Claude Code, Codex, OpenCode, Copilot CLI/Desktop, VS Code Copilot, Cursor, Cursor Agent, Cline, Aider, Gemini, Kimi, ForgeCode, Kiro, CodeBuddy, Antigravity) into a single internal model (`ClaudeMessage` / `ClaudeSession` / `ClaudeProject`). Each provider module exposes the same contract (`detect` / `scan_projects` / `load_sessions` / `load_messages`), the parsing is already GUI-independent (serde, simd-json, memmap2, walkdir, rayon, rusqlite, chrono, aho-corasick, memchr), and a `webui-server` Cargo feature plus a `--export` headless mode already prove the core runs without the webview. There is no database today — everything is read-from-disk on demand with an in-memory search cache.

The motivating problem: agents delete local history on a timer (Claude Code at 30 days) and history is siloed per machine. We want a durable, cross-machine, searchable personal archive. This change builds the MVP — durable archive plus a search/query API — reusing the existing parsers. Semantic search (pgvector) and an MCP agent-context server are deliberately deferred but the data model is designed to accept them.

Constraints carried from the project: pnpm for the frontend, Rust workspace, the existing desktop app must stay green (`cargo test --test-threads=1`, `clippy -D warnings`, `cargo fmt --check`), and `history-core` must not depend on `tauri`. Deployment target is a self-hosted Postgres + hub on one always-on Tailscale tailnet node; daemons run on each machine.

## Goals / Non-Goals

**Goals:**

- Extract the existing parsers into a tauri-free `history-core` library shared by the desktop app and a new sync daemon, with no behavior change to the desktop app.
- A per-machine push daemon that backfills then incrementally syncs local history to a central hub, with crash-safe checkpointing, at-least-once delivery, and cumulative (never-delete) semantics.
- A central hub (the only DB-touching component) that ingests idempotently into Postgres and serves full-text search and browse queries to any machine over the tailnet.
- A Postgres schema that keeps both normalized columns and the raw original record, supports Postgres FTS now, and is pgvector-ready without a future breaking migration.

**Non-Goals:**

- pgvector embeddings and semantic search (Phase 2).
- An MCP server for agent-context injection (Phase 3).
- Pointing the existing React viewer at the hub (later; endpoint shapes are kept compatible to make it cheap).
- Cross-machine logical project merging by git remote.
- Multi-user, public-internet exposure, or cloud-managed Postgres. Trust is the tailnet plus a per-machine bearer token.

## Decisions

### D1: Workspace restructure with `history-core` as a tauri-free library

Convert the fork into a Cargo workspace: `crates/history-core` (lib), `crates/sync-daemon` (bin), `crates/hub` (bin), plus the existing `src-tauri` which now depends on `history-core`. Move `src-tauri/src/{models, providers, utils.rs}` and the GUI-independent parse logic from `export.rs` into `history-core`; leave the `#[tauri::command]` wrappers in `src-tauri` as thin adapters that call the library.

- **Why:** One parser for all providers, reused by daemon and app, with no format drift. Tauri apps already compile to a lib crate, so the parsers are nearly lib-shaped already; the main work is severing `#[tauri::command]`/feature-gate coupling.
- **Alternatives considered:** (a) Depend on the whole existing app lib crate — rejected: drags `tauri` into the daemon. (b) Reimplement parsing in Go/TS for the pipeline — rejected: re-solves 16 evolving formats and guarantees drift. (c) Vendor/submodule the parsers into a separate repo — rejected in brainstorming in favor of a single fork workspace for lower friction.
- **Risk control:** The extraction is behavior-neutral and gated by the desktop app's existing test/clippy/fmt suite plus per-provider golden tests.

### D2: Push daemon (not central poll), hub owns Postgres

Each machine runs a daemon that reads local history and POSTs batches to the hub; the hub is the sole holder of DB credentials and the sole writer.

- **Why:** Works when other machines are offline, is near-real-time, and keeps DB credentials and schema knowledge in exactly one place. Daemons carry only a URL + token, so rotating credentials or migrating the schema never touches every machine.
- **Alternatives considered:** central poller over the existing `--serve` API (needs every machine reachable and running a server at poll time); daemons writing directly to Postgres via sqlx (spreads credentials and schema-lockstep across machines, harder migrations).

### D3: Idempotent ingest keyed by `(machine_id, provider, session_id, message_key)`

`message_key` is the provider message UUID when present, else a content-derived hash. Messages are treated as immutable once written: `INSERT ... ON CONFLICT DO NOTHING`; session/project aggregates are updated on every ingest.

- **Why:** Enables at-least-once delivery from the daemon (retries are safe) and tolerates providers without stable UUIDs. Aggregates are maintained at write time so browse queries never scan all messages.
- **Alternatives considered:** rely solely on provider UUID (breaks for UUID-less providers); recompute aggregates on read (expensive at archive scale).

### D4: Store normalized + raw + FTS together; defer embeddings to a side table

Each message row holds normalized columns, `content` JSONB (normalized), `raw` JSONB (exact original), `search_text` (flattened plaintext), and `text_search tsvector GENERATED ALWAYS AS (to_tsvector('simple', search_text)) STORED` with a GIN index. Embeddings will live later in a separate `message_embeddings(message_id, model, embedding vector(N))` table.

- **Why:** Raw fidelity is essential precisely because the source gets deleted — once it's gone, normalized-only would be lossy forever and re-embedding/reprocessing would be impossible. A generated tsvector keeps FTS correct without app-side maintenance. A side table for embeddings avoids a breaking `ALTER` on the hot `messages` table and supports multiple embedding models.
- **Alternatives considered:** normalized-only (lossy, blocks reprocessing); an `embedding` column on `messages` now (requires pgvector immediately and a wide table); an external search engine (extra infra; Postgres FTS is enough for MVP and keeps the pgvector path in one system).

### D5: `search_text` computed by the daemon, tsvector by Postgres

The daemon (via `history-core`) produces the flattened plaintext `search_text` by porting the existing `contentExtractor`/`extractSearchableText` flattening; Postgres derives the `tsvector` from it via a generated column.

- **Why:** Reuses the already-correct flattening logic and keeps tokenization/ranking in the database where search runs.

### D6: Rust + axum + sqlx for the hub; reqwest + notify for the daemon

The hub uses axum and sqlx (Postgres, compile-time-checked queries with offline metadata committed for CI). The daemon uses reqwest for delivery and notify for debounced file-watching, plus a periodic safety-net rescan.

- **Why:** Stays in one language with `history-core`; sqlx offline keeps CI hermetic; a watcher gives low latency while the periodic rescan bounds worst-case staleness if an event is missed.

### D7: Auth and transport = tailnet + per-machine bearer token

All hub endpoints except `/healthz` require a bearer token; the hub maps token → `machine_id` from config. Network reachability and transport encryption are provided by Tailscale; TLS termination can be added later.

- **Why:** Minimal, sufficient for a personal single-user system on a trusted tailnet. Keeps the MVP free of certificate/identity-provider complexity while still rejecting unauthenticated access.

## Risks / Trade-offs

- **Extraction refactor causes upstream merge friction** → Keep the move behavior-neutral and mechanical, mirror the directory layout under `history-core`, and gate with golden tests so a future rebase on upstream is a move-conflict, not a logic-conflict.
- **Provider format changes upstream break parsing silently** → `history-core` golden tests per provider; the daemon's safety-net rescan plus raw-fidelity storage means a parser fix can be re-applied to already-archived `raw` records later.
- **At-least-once delivery duplicates rows** → Idempotent upsert on the composite key (D3); double-POST is verified by an ingest idempotency test.
- **Content-derived `message_key` collisions for UUID-less providers** → Hash over the full normalized record plus session-local sequence to make collisions practically impossible; covered by a dedup test for a UUID-less provider.
- **`raw` JSONB inflates storage** → Acceptable: conversation text is small and compressible; raw fidelity is the core value (sources get deleted). Revisit with TOAST/compression only if it becomes a problem.
- **Daemon checkpoint corruption loses sync position** → Checkpoint writes are atomic (temp + rename); on unreadable checkpoint the daemon falls back to a full rescan, which is safe because ingest is idempotent.
- **Hub is a single point of failure / the tailnet node is down** → Daemons retry with backoff and persist un-acknowledged work in the checkpoint, so archiving resumes when the hub returns; no data is lost, only delayed.

## Migration Plan

1. **Workspace + library extraction (D1).** Introduce the `[workspace]`, create `history-core`, move parsers, wire `src-tauri` to depend on it. Land only when the desktop app's full validation suite is green and per-provider golden tests pass. This is independently shippable and user-invisible.
2. **Schema + hub ingest (D3, D4).** Add `migrations/`, stand up the `hub` with `/ingest` and `/healthz` against a throwaway Postgres in CI. Verify idempotency and raw round-trip.
3. **Sync daemon (D2).** Build backfill → incremental → checkpoint, delivering to the hub. Verify with a temp history dir against a mock hub, then end-to-end against a real hub + Postgres in CI.
4. **Search + browse API.** Add `/search` and browse endpoints; verify FTS correctness and stable pagination.
5. **Deploy.** Run migrations + hub on the always-on tailnet node; install the daemon (launchd/systemd) on each machine with a hub URL + token; let backfill run once, then it stays incremental.

**Rollback:** Each step is additive. The hub/daemon/Postgres are new and isolated — disabling the daemon and stopping the hub fully reverts the new system with no effect on the desktop app. The workspace refactor (step 1) is reverted by restoring the pre-workspace `src-tauri` layout; because it is behavior-neutral, reverting carries no data or behavior risk.

## Open Questions

- Checkpoint store format: a small SQLite file vs. a JSON file in the daemon state dir. Leaning SQLite for atomic per-file updates at scale; final choice during step 3.
- `machine_id` shape: persisted UUID vs. hostname-derived stable id. Leaning persisted UUID (plus hostname for display) so renames don't fork identity.
- CI Postgres: testcontainers vs. a CI service container. Leaning a CI service container for speed, with sqlx offline metadata committed so non-DB builds don't need Postgres.
- Whether the daemon ports the full `extractSearchableText` flattening into `history-core` now or computes a simpler `search_text` first and enriches later. Leaning full port to avoid a second pass.
