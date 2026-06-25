## 1. Workspace + history-core extraction (history-core-library)

- [x] 1.1 Add a root `[workspace]` `Cargo.toml` with members `crates/history-core` and `src-tauri` (sync-daemon/hub members added when those crates are created in groups 3–4); lockfile moved to root; profiles + lints lifted to `[workspace.*]`; desktop app builds unchanged
- [x] 1.2 Scaffold `crates/history-core` with no `tauri`/`tauri-*` dependency; deps set to the move-set's actual footprint (serde, serde_json, simd-json, memmap2, walkdir, rayon, rusqlite, chrono, uuid, memchr, dirs, base64, comrak, once_cell, log; dev: insta, pretty_assertions, serial_test, tempfile)
- [x] 1.3 Move `src-tauri/src/models/` into `history-core` and re-export `ClaudeMessage`/`ClaudeSession`/`ClaudeProject` as the crate's public model API (incl. `NativeRenameResult` lifted from `commands::session::rename`, and the orphan-rule `TryFrom<RawLogEntry>` impl)
- [x] 1.4 Move `src-tauri/src/providers/`, `utils.rs`, `cli_args.rs`, `fs_utils.rs`, and the pure Antigravity state logic into `history-core`, severing `#[tauri::command]`/feature-gate coupling (thin wrappers + re-export shims left behind in `src-tauri`)
- [x] 1.5 Port the GUI-independent parse/flatten logic from `export.rs` plus the `contentExtractor`/`extractSearchableText` flattening into `history-core` as a `search_text` builder — DONE alongside its consumer (group 4): `history_core::search_text::search_text(&ClaudeMessage)` recursively flattens content/tool_use/tool_use_result to plaintext, skipping metadata keys; 5 unit tests; the daemon computes it per message. `export.rs` block-extraction already moved to core in group 1
- [x] 1.6 Expose the stable headless API (`detect_providers`, `scan_projects`, `load_sessions`, `load_messages`) returning normalized models (preserved via the providers module's existing contract, now under `history_core::providers`)
- [x] 1.7 Update `src-tauri` to depend on `history-core`; keep `#[tauri::command]` functions as thin adapters that call the library (modules re-exported under original `crate::` paths so consumers compile unchanged)
- [x] 1.8 Add per-provider golden tests in `history-core` using existing fixtures (parse → normalized snapshot), and a stability test (parse-twice equality) — DONE: existing insta golden/snapshot + provider tests run in `history-core` (incl. the 61 moved Claude loader tests), plus a new `tests/stability_test.rs` asserting `load_messages` is deterministic across re-parses (underpins the daemon's content-hash dedup keys)
- [x] 1.9 Verify `cargo tree -p history-core` shows no `tauri`, `cargo build -p history-core` succeeds standalone, and the desktop suite stays green — VERIFIED: zero tauri in dep graph, standalone build ok, `clippy --workspace --all-targets --all-features -D warnings` clean, `fmt --check` clean, tests green (357 default + 399 webui-server + 332 history-core, 0 failures)

## 2. Postgres schema + migrations (archive-ingestion)

- [x] 2.1 Add a top-level `migrations/` directory (`0001_initial_schema.sql`). NOTE: the runtime `sqlx` wiring + committed offline metadata move to group 3 (3.1) where the hub's `query!` macros first exist — offline metadata cannot be generated before there are queries
- [x] 2.2 Write the `machines` migration (machine_id PK, hostname, os, first_seen, last_seen) — validated against PG 18
- [x] 2.3 Write the `projects` migration (identity PK, UNIQUE(machine_id, provider, project_path), name, storage_type, aggregates, timestamps) — validated
- [x] 2.4 Write the `sessions` migration (identity PK, UNIQUE(machine_id, provider, session_id), project FK, file_path, entrypoint, summary, aggregates, flags, timestamps) — validated
- [x] 2.5 Write the `messages` migration: normalized columns, `content`/`raw` JSONB, `search_text`, `text_search` GENERATED `tsvector` STORED, `content_hash`, `seq`. Dedup key normalized to UNIQUE(session_id_fk, message_key) — the session FK encodes (machine_id, provider, provider_session_id), so this enforces the spec's logical (machine_id, provider, session_id, message_key) identity. Validated: FTS match + dup rejection
- [x] 2.6 Add indexes: GIN on `text_search`, btree on (machine_id, provider), timestamp, and sessions(project_id); the session-FK lookup is served by the UNIQUE(session_id, message_key) index — validated present
- [x] 2.7 Confirm the schema requires no `vector` extension (proven: applied on a DB without `vector` installed); future `message_embeddings(message_id, model, embedding vector(N))` documented in-file, not created
- [x] 2.8 Add a test that applies migrations to an empty Postgres and asserts all tables/indexes exist and migrations are re-appliable — DONE in the hub crate (`crates/hub/tests/migration_test.rs`): applies + re-applies `MIGRATOR` (idempotent), asserts all 4 tables + the GIN index exist. Also manually validated earlier via psql (FTS match, dup rejection, no vector ext)

## 3. Hub service: ingest (archive-ingestion)

- [x] 3.1 Scaffold `crates/hub` (axum + sqlx + tokio) with TOML/env config (database_url, bind_addr, token→machine_id), a `PgPool`, and `MIGRATOR` applying `migrations/` on startup. sqlx offline metadata (`.sqlx/`) committed for hermetic CI builds (SQLX_OFFLINE) — closes the 2.1 deferral
- [x] 3.2 Bearer-token auth via the `AuthedMachine` extractor (missing/invalid token → 401; token → machine_id); ingest also rejects a body machine_id that mismatches the token
- [x] 3.3 Wire types in a dedicated `crates/protocol` (`archive-protocol`): `IngestBatch`/`MachineInfo`/`IngestProject`/`IngestSession`/`IngestMessage` (carries `raw` + `search_text`) and `IngestResponse` (inserted/skipped counts) — shared by hub and the future daemon, keeping history-core pure
- [x] 3.4 `POST /v1/ingest`: validate, upsert machine/projects/sessions, insert messages `ON CONFLICT (session_id, message_key) DO NOTHING`, recompute session+project aggregates cumulatively, all in one transaction. NOTE: `message_key` is computed daemon-side (uuid else content hash) and sent on the wire; `content_hash` column reserved/NULL for now (group 4 daemon may populate it)
- [x] 3.5 `GET /v1/healthz` (unauthenticated) — `SELECT 1` connectivity check, 200/503
- [x] 3.6 Integration tests against local Postgres — all 8 green: valid ingest counts, missing→401, invalid→401, unknown-session→400 with full rollback (no partial write), double-POST idempotent (no dupes), raw JSONB round-trips, UUID-less dedup, cumulative aggregate update on re-ingest

## 4. Sync daemon (history-sync-daemon)

> PRE-REQ (done): extracted Claude Code's loader into `history_core::providers::claude` and added a unified `providers::{scan_all_projects, load_sessions, load_messages}` dispatch + `history_core::search_text` (closes 1.5), so the daemon loads ALL providers via the shared parser.

- [x] 4.1 Scaffold `crates/sync-daemon` (reqwest + tokio) depending on `history-core` + `archive-protocol`; TOML/env config (hub URL + token + tuning), structurally no DB credentials. NOTE: `notify` file-watching deferred — the MVP uses a periodic full rescan (4.7), which is simpler and equally correct given the hub dedups
- [x] 4.2 Stable machine identity: UUID persisted atomically in the state dir (default `~/.claude-history-sync/machine_id`) + hostname, attached to every batch's `MachineInfo`
- [x] 4.3 Crash-safe checkpoint store (`checkpoint.json`, atomic temp+rename) recording per session file: size, mtime, message_count, last_synced_ms. Change detection is size+mtime (a content hash per file was unnecessary given size+mtime + hub idempotency)
- [x] 4.4 At-least-once batched delivery to `/v1/ingest` (chunked at `batch_max_messages`, default 500) with exponential backoff retry on 5xx/transport (4xx = permanent); a file's checkpoint advances only after every chunk is acked
- [x] 4.5 Backfill: enumerate via `history-core` dispatch, scan all projects/sessions/messages, deliver, checkpoint after ack; resumable (checkpoint saved per session; an interrupted run re-sends only un-acked sessions, hub dedups)
- [x] 4.6 Incremental sync via change-detect (size/mtime) + full re-parse + idempotent re-send. NOTE: byte-offset "parse only appended lines" and `notify` watching are documented future optimizations — the chosen mechanism still meets the outcome (appended/rewritten messages sync, no dups via hub dedup)
- [x] 4.7 Periodic safety-net full rescan (the run loop) + cumulative semantics enforced: a vanished/truncated local file is simply not re-seen; the daemon issues only ingests, never deletes
- [x] 4.8 8 integration tests (temp `$HOME` fixture + mock hub, `#[serial]` + `--test-threads=1`), all green: cold-start delivers once, checkpoint survives restart (no redundant delivery), appended messages sync, failed delivery not checkpointed + resends (at-least-once), machine id stable, deleted source leaves archive intact, search_text computed+delivered, config loads from url+token without DB

## 5. Search + browse API (archive-search-api)

- [x] 5.1 `GET /v1/search` via `websearch_to_tsquery` over `text_search`, ranked by `ts_rank` with `ts_headline` snippets; optional provider/machine(hostname)/project(name|path)/from/to filters via the static `$n IS NULL OR …` idiom (keeps queries compile-checked); session+project+machine context joined in
- [x] 5.2 Browse endpoints `GET /v1/projects`, `GET /v1/sessions` (machine/provider/project filters), `GET /v1/sessions/{id}/messages` (by surrogate session id, ordered by seq→timestamp→id); a new `Authenticated` extractor lets reads span all machines
- [x] 5.3 Bearer auth on all read endpoints (401 without a valid token) + bounded pagination (`Page`: default 50, max 200) with id-tiebroken stable ordering
- [x] 5.4 7 read integration tests, all green: ranked results w/ context, filters narrow, empty→200, projects provenance+aggregates, messages in seq order, unauthenticated→401, stable paging (no dup/drop across pages). `.sqlx` offline metadata regenerated (12 queries)

## 6. End-to-end + deployment

- [x] 6.1 e2e coverage: `crates/sync-daemon/tests/e2e_test.rs` drives the WHOLE pipeline in-process (fixture `~/.claude` → real daemon sync → real hub → Postgres → `GET /v1/search` finds it), plus a new `.github/workflows/archive-tests.yml` (Postgres service) running fmt/clippy/tests for the four archive crates. The `cd src-tauri` desktop CI is unaffected
- [x] 6.2 Deployment guide `docs/archive/deployment.md`: Postgres setup, hub build + TOML config (token→machine_id) + systemd, daemon build + config + launchd/systemd install, end-to-end verification via curl, and current MVP limitations
- [x] 6.3 README "Cross-Machine History Archive" section + CHANGELOG `[Unreleased]` entry. Workspace target-dir moved to repo root → fixed `rust-tests.yml` cache path and the `src-tauri/target` artifact paths in `server-release.yml`/`updater-release.yml` (flagged for verification at next desktop release). Frontend (`src/`) provably unchanged (0 files touched); desktop Rust validation green
