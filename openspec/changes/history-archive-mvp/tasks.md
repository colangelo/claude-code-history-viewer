## 1. Workspace + history-core extraction (history-core-library)

- [x] 1.1 Add a root `[workspace]` `Cargo.toml` with members `crates/history-core` and `src-tauri` (sync-daemon/hub members added when those crates are created in groups 3–4); lockfile moved to root; profiles + lints lifted to `[workspace.*]`; desktop app builds unchanged
- [x] 1.2 Scaffold `crates/history-core` with no `tauri`/`tauri-*` dependency; deps set to the move-set's actual footprint (serde, serde_json, simd-json, memmap2, walkdir, rayon, rusqlite, chrono, uuid, memchr, dirs, base64, comrak, once_cell, log; dev: insta, pretty_assertions, serial_test, tempfile)
- [x] 1.3 Move `src-tauri/src/models/` into `history-core` and re-export `ClaudeMessage`/`ClaudeSession`/`ClaudeProject` as the crate's public model API (incl. `NativeRenameResult` lifted from `commands::session::rename`, and the orphan-rule `TryFrom<RawLogEntry>` impl)
- [x] 1.4 Move `src-tauri/src/providers/`, `utils.rs`, `cli_args.rs`, `fs_utils.rs`, and the pure Antigravity state logic into `history-core`, severing `#[tauri::command]`/feature-gate coupling (thin wrappers + re-export shims left behind in `src-tauri`)
- [ ] 1.5 Port the GUI-independent parse/flatten logic from `export.rs` plus the `contentExtractor`/`extractSearchableText` flattening into `history-core` as a `search_text` builder — DEFERRED: `export.rs` (incl. block-extraction/flatten) is moved to core; the dedicated public `search_text` builder is intentionally left for group 4/5 where the daemon/hub consume it (avoids building a speculative API shape)
- [x] 1.6 Expose the stable headless API (`detect_providers`, `scan_projects`, `load_sessions`, `load_messages`) returning normalized models (preserved via the providers module's existing contract, now under `history_core::providers`)
- [x] 1.7 Update `src-tauri` to depend on `history-core`; keep `#[tauri::command]` functions as thin adapters that call the library (modules re-exported under original `crate::` paths so consumers compile unchanged)
- [ ] 1.8 Add per-provider golden tests in `history-core` using existing fixtures (parse → normalized snapshot), and a stability test (parse-twice equality) — PARTIAL: existing insta golden/snapshot + provider tests now run in `history-core` and pass (332 tests); the explicit new parse-twice stability test is a small recommended follow-up
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

- [ ] 4.1 Scaffold `crates/sync-daemon` (reqwest + notify + tokio) depending on `history-core`; load config (hub URL + bearer token + tuning), refusing to require DB credentials
- [ ] 4.2 Establish stable machine identity: persist a UUID in the state dir (`~/.claude-history-sync/`) and attach it + hostname to every batch
- [ ] 4.3 Implement the crash-safe checkpoint store (atomic temp+rename) recording per session file: size/offset, message_count, content_hash, last_synced_at
- [ ] 4.4 Implement at-least-once batched delivery to `/v1/ingest` (≈500 msgs or ≈1MB per batch) with retry/backoff; advance a file's checkpoint only after ack
- [ ] 4.5 Implement backfill: enumerate all providers via `history-core`, scan everything, deliver, checkpoint; make it resumable from the last ack
- [ ] 4.6 Implement incremental sync: append-offset parsing for JSONL, full re-parse + key-diff for rewritten/SQLite providers, debounced `notify` watch on provider roots
- [ ] 4.7 Implement the periodic safety-net full rescan and enforce cumulative semantics (local deletions/truncations never emit deletes to the hub)
- [ ] 4.8 Add integration tests with a temp history dir + mock hub: cold-start delivers once, interrupted backfill resumes, appended lines sync via offset, rewritten session re-diffs to only-new, checkpoint survives restart, transient failure retried exactly-once, machine id stable across restarts, deleted source leaves archive intact

## 5. Search + browse API (archive-search-api)

- [ ] 5.1 Implement `GET /v1/search` using `websearch_to_tsquery` over `text_search`, ranked, with provider/machine/project/time filters and limit/offset; return matches with session+project context
- [ ] 5.2 Implement browse endpoints `GET /v1/projects`, `GET /v1/sessions?project=`, `GET /v1/sessions/:id/messages` mirroring the webui-server response shapes, in stable order
- [ ] 5.3 Apply bearer auth to all read endpoints and implement bounded, stable pagination
- [ ] 5.4 Add integration tests: ranked query results, filters narrow results, empty result set is well-formed 200, projects list carries provenance + aggregates, session messages return in order, unauthenticated read → 401, paging is stable (no drops/dupes)

## 6. End-to-end + deployment

- [ ] 6.1 Add an e2e CI job: spin Postgres + hub, run the daemon against a fixture machine dir, assert backfilled content is searchable via `/v1/search`
- [ ] 6.2 Add deployment docs/scripts: run migrations + hub on the always-on tailnet node, and install the daemon (launchd/systemd) on a machine with a hub URL + token
- [ ] 6.3 Update README/CHANGELOG for the new workspace layout and the archive system; confirm frontend (pnpm) and desktop validation remain unaffected
