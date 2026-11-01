# Tasks

## 1. Hub
- [x] 1.1 Tests first: `crates/hub/tests/token_scope_test.rs` gains read-only tests (reads every
      read route; refused on ingest, journal writes, alias create/delete, and a POST to an
      unregistered path; full and ingest-only tokens unchanged) plus config unit tests (flag
      collected; both flags fail the load). Red before the code (c1674822: 2 of 11 failed), green after; an
      extractor scope-table unit test in `auth.rs`. Mutation-checked: no layer, ingest extractor
      accepting read-only, read refusing read-only — each fails a test.
- [x] 1.0 Pin (manager-lab, from infra's gap at efc259c8): `journal_writes_stay_bearer_only` — a
      trusted `Tailscale-User-Login` header with no bearer gets 401 on every `AuthedMachine` route
      and on ingest, while the same header still reads `/v1/projects` (control). Green on the
      current code; it guards the behaviour, it does not change it.
- [x] 1.2 `TokenEntry.read_only` (default false), `HubConfig::read_only_tokens()`, the both-flags
      check in `HubConfig::load`, `AppState::with_read_only_tokens`.
- [x] 1.3 Auth: `Scope::{Ingest, Write, Read}`; `IngestMachine`/`AuthedMachine` refuse read-only;
      `Authenticated` accepts it; router layer refuses it on any method but GET/HEAD/OPTIONS.
- [ ] 1.4 Gate: fmt, clippy `-D warnings` (Rust 1.99), archive-crate tests.

## 2. Launcher
- [x] 2.1 `scripts/cchv-launch.sh`: one read-only entry per reader from
      `kv/agents/cchv-readers/<reader>`; shape checks; skip malformed; dedupe machine ids across
      guests and readers. Sandbox-tested with a stubbed bao: valid, malformed, duplicate, none.
- [x] 2.2 `docs/archive/deployment.md`: the token kinds and the readers path.

## 3. Review and release
- [ ] 3.1 security reviews the change (two-vendor rule).
- [ ] 3.2 Release under cchv-deploys-ship-when-ready; infra swaps hub + launcher.
- [ ] 3.3 infra mints `kv/agents/cchv-readers/agents-blog`, grants the hub AppRole read on
      `kv/agents/cchv-readers/*` and the blog sink read on its own leaf. Read-back: blog token GET
      `/v1/projects` 200; POST `/v1/ingest` and `/v1/journal/entries` 401; a Mac token unchanged.
