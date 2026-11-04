## 1. Hub: excluded paths (D1)

- [ ] 1.1 `config.rs`: `[journal]` section with `exclude_paths: Vec<String>` and `home: Option<String>`; `deny_unknown_fields` like its siblings; validation per D1 (absolute or `~/`; `~/` needs `home`); compile to the `LIKE` patterns once, built-in defaults first. Unit tests: escaping of `%`/`_`/`\`, `*` → `%`, `~/` expansion, refusal without `home`, defaults always present.
- [ ] 1.2 One helper that renders the exclusion clause for a given bind index. `health.rs` (journal health query) and `journal.rs` (`pending`) both use it with the same bound array.
- [ ] 1.3 Route test (DB): a group under `/private/tmp/x` and one under a configured `~/.local/state/scratch/*` are absent from `/v1/journal/pending` AND never make `/v1/healthz/journal` stale, while a normal group in the same day still does.
- [ ] 1.4 Extend the existing "Health agrees with pending" test so it runs with a non-empty exclusion list.

## 2. Hub: client groups (D2)

- [ ] 2.1 Health query returns `is_client` per group; body adds `client_undistilled`, `client_groups_page`, and `client` per group. Paths stay hidden as today.
- [ ] 2.2 Route tests (DB): enforce on + no `[[client_readers]]` → a stale client group is counted, status `ok`, 200; enforce on + one client reader → `stale`, 503; enforce off → `stale`, 503.

## 3. Hub: paused (D3)

- [ ] 3.1 `migrations/0019_distiller_tick_reason.sql` (nullable `reason text`). Update `.sqlx` only if a `query!` macro changes (`cargo sqlx prepare` rewrites the shared dir; prefer runtime queries as the file already does).
- [ ] 3.2 `journal.rs` ticks handler: accept `paused` with a required reason (1 to 500 chars), reject a reason on other modes; 400 messages name the field.
- [ ] 3.3 Health: newest tick `paused` → `status: "paused"`, 200, `paused_reason`, `paused_since`; outranks `stale` and `no_tick`. A later forward tick clears it.
- [ ] 3.4 Route tests (DB): pause → 200 paused with reason, even with a stale group and a `max_tick_age_secs` that would say `no_tick`; then a forward tick → back to `stale` 503; bad payloads → 400.

## 4. Distiller (D4, D5, D6)

- [ ] 4.1 `--pause --reason TEXT`: posts the paused tick, exits 0; no model credential, no pending read. `--reason` required.
- [ ] 4.2 Forward drain loop per D4 (`--budget-secs`, default 2700; stop on empty, budget, or a zero-success batch). Backfill unchanged.
- [ ] 4.3 `_claude_generate`: `ENABLE_CLAUDEAI_MCP_SERVERS=false` always; `--settings ~/.config/claude/direct-noconnectors.settings.json` when the file exists. Same env for any other model CLI started.
- [ ] 4.4 Tests in the distiller's existing test suite (find it with `rg -l cchv-distill tests scripts`): argv and env of the claude call (with and without the settings file), the drain loop's three stop conditions with a fake hub, `--pause` posting exactly one tick and never touching pending.

## 5. Spec, docs, gate

- [ ] 5.1 Spec deltas under `specs/journal-health/` and `specs/journal-entries/` match what was built.
- [ ] 5.2 `docs/archive/deployment.md`: the `[journal]` config keys and `--pause`, in the section that documents the journal check (read it first; keep its style).
- [ ] 5.3 Gate: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` (toolchain 1.99.0), hub tests `--test-threads=1`, distiller tests. DB tests need `TEST_DATABASE_URL` on a local Postgres; if the seat cannot reach it, say so and the lead runs them.
