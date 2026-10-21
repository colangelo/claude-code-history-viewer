# Tasks

## 1. Hub
- [x] 1.1 Tests first: `crates/hub/tests/token_scope_test.rs` (5 tests) and two config unit tests; the two scope tests failed before the auth change (red), all pass after.
- [x] 1.2 `TokenEntry.ingest_only` (default false), `HubConfig::ingest_only_tokens()`, `AppState::with_ingest_only_tokens`.
- [x] 1.3 Auth: `IngestMachine` for ingest; `AuthedMachine` and `Authenticated` refuse ingest-only tokens.
- [x] 1.4 Gate: fmt, clippy `-D warnings` (Rust 1.99), archive-crate tests.

## 2. Launcher
- [x] 2.1 `scripts/cchv-launch.sh`: render agent1's ingest-only entry from `kv/agents/cchv-agent1` (64-hex shape check), skip it when missing or malformed. Sandbox-tested: valid / malformed / missing.

- [x] 2.2 One ingest-only entry per guest from `kv/agents/cchv/<guest>` (fields `token`, `machine_id`; label = guest name, `^[a-z0-9][a-z0-9-]{0,31}$`), listed via `kv/metadata/agents/cchv/`; each malformed or unreadable guest is skipped alone. The legacy `kv/agents/cchv-agent1` entry is still read, deduplicated by machine id, until infra moves agent1. Sandbox-tested: migrated (dedupe), legacy-only, malformed set (name, token, id, duplicate), none.
- [x] 2.3 After the migration: drop the legacy path and `AGENT1_LEGACY_MACHINE_ID`. infra moved agent1 to `kv/agents/cchv/agent1` and deleted `kv/agents/cchv-agent1` (2026-10-21). Sandbox-tested: agent1 only, agent1 + a second guest, list fails.

## 3. Release and rollout (ac's call, asks a1021-01)
- [ ] 3.1 Release cchv-v0.26.0 with the Linux x86_64 sync-daemon attached; infra swaps the hub and reinstalls the launcher.
- [ ] 3.2 Infra: `kv/agents/cchv-agent1`, hub AppRole read on it, ACL `tag:ai-agent -> m4m:8788`, Gatus excludes, daemon on agent1.
- [ ] 3.3 Read-back from agent1: `/v1/healthz` 200; unauthenticated and agent1-token reads 401; agent1-token journal POST 401; ingest 200; a Mac token still reads 200.
