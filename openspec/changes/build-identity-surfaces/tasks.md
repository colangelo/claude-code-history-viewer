## 1. Hub: version on `/v1/healthz` (#39)

- [x] 1.1 `crates/hub/src/health.rs::healthz` — add `"version": env!("CARGO_PKG_VERSION")`
      to both the `db: up` and `db: down` bodies. Verify: `read_test.rs`
      `healthz_reports_ok_unauthenticated` asserts `version == env!("CARGO_PKG_VERSION")`
      and that it is exact semver (`\d+\.\d+\.\d+`), and the db-down test asserts the
      field is present there too.

## 2. Hub: tick identity (#40, hub half)

- [x] 2.1 `migrations/0009_distiller_ticks_identity.sql` — `ALTER TABLE distiller_ticks
      ADD COLUMN distiller_version text, ADD COLUMN distiller_blob text` (nullable, no
      default), with a header comment saying why null is a reading. Verify:
      `migration_test.rs` applies cleanly on a 0008 database and the columns exist.
- [x] 2.2 `journal.rs::TickPayload` — `#[serde(default)] distiller_version:
      Option<String>`, `distiller_blob: Option<String>`; `record_tick` rejects a present
      blob that is not `^[0-9a-f]{40}$` with `BadRequest` naming `distiller_blob`, and
      binds both into the INSERT. Verify: `read_test.rs` — POST with both fields → 200
      and the row holds them; POST with neither → 200 (unchanged behaviour); POST with a
      39-char / uppercase / non-hex blob → 400 whose message contains `distiller_blob`
      and no row is written.
- [x] 2.3 `health.rs` — `TickSummary` + the LATERAL query gain `distiller_version`,
      `distiller_blob`; `JournalHealthResponse` gains `hub_version: &'static str`
      (`CARGO_PKG_VERSION`), `last_tick_distiller_version`, `last_tick_distiller_blob`.
      Verdict logic untouched. Verify: `read_test.rs` — after a tick with identity the
      journal health body carries all three; after a tick without identity the two
      `last_tick_*` are null and `last_tick_at` is set; with an empty table all three
      tick fields are null and `hub_version` is still present.
- [x] 2.4 `cargo test -p hub -- --test-threads=1`, `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all -- --check` all green. Verify:
      exit codes.

## 3. Distiller identity (#40, script half)

- [x] 3.1 `scripts/cchv-distill.py` — add `DISTILL_VERSION = "<current>"  # sync-version`
      near the top, and `script_blob_id() -> str` = git blob sha1 of
      `Path(__file__).read_bytes()`. Verify: `test_cchv_distill.py` —
      `script_blob_id()` equals `git hash-object scripts/cchv-distill.py` (subprocess),
      and `DISTILL_VERSION` equals `package.json`'s `version`.
- [x] 3.2 `main()` — first log line, before secrets resolution:
      `cchv-distill {DISTILL_VERSION} blob={blob[:12]} mode={forward|backfill|dry-run}`.
      Verify: a test capturing `log()` sees that line first in `--dry-run`, and the
      existing `test_dry_run_records_no_tick` still passes.
- [x] 3.3 `Hub.record_tick` — payload gains `distiller_version` and `distiller_blob`
      (full 40 hex). Verify: `test_tick_record_states_its_mode_and_work_list_size`
      extended to assert both fields; `test_a_failed_tick_record_never_costs_the_run`
      unchanged and green.
- [x] 3.4 `uv run scripts/test_cchv_distill.py` (or the repo's invocation) green. Verify:
      exit code.

## 4. Release tooling: the fifth sync-version target

- [x] 4.1 `scripts/sync-version.cjs` — rewrite the `DISTILL_VERSION = "…"  # sync-version`
      line in `scripts/cchv-distill.py`; **exit 1 with a clear message if the marker is
      not found**; update the header comment's target list. Verify: run it on a throwaway
      version, `git diff --stat` shows exactly three files changed (Cargo.toml,
      tauri.conf.json, cchv-distill.py); remove the marker in a scratch copy and confirm
      the non-zero exit; restore.
- [x] 4.2 `AGENTS.md` — § Version Management diagram and the `just sync-version`
      comments list the script as a target; § Release Process Phase 3 explicit `git add`
      line gains `scripts/cchv-distill.py` and its comment explains that Guard 1 is what
      catches a forgotten fifth target (this is the case the comment already describes
      hypothetically — make it concrete). Verify: `grep -n "cchv-distill.py" AGENTS.md`
      hits the staging line and the diagram.

## 5. Docs and deploy contract

- [ ] 5.1 `docs/archive/deployment.md` §2b — the swap proof is now
      `curl -s $HUB/v1/healthz | jq -r .version` == released version (exact string
      compare), replacing route archaeology; §3c — after a distiller reinstall, the
      first tick's log line and `GET /v1/healthz/journal` `last_tick_distiller_blob`
      must equal `git rev-parse <tag>:scripts/cchv-distill.py`. State that a version
      skew between hub and distiller right after a hub swap is expected **until the
      next tick**, and that a version-only skew shows as a one-line
      `git diff <old> <new> -- scripts/cchv-distill.py`. Verify: both sections name
      the exact commands.
- [ ] 5.2 Note for the `cchv-deploy` skill (CONTEXT repo — not edited from here): its
      § swap-proof probe should prefer the `version` field from this release on; its
      Phase 2 staging list gains the script. Verify: the note is in the release relay
      and recorded as a comment on #39 so it has an owner.

## 6. Release + deploy (`cchv-deploy` skill)

- [ ] 6.1 Quality gate per `AGENTS.md` Phase 1 (pnpm tsc/vitest/lint, cargo test/clippy/
      fmt, i18n validate). Verify: all exit 0 (Rust via CI if this Mac's toolchain
      cannot — see memory `local-clippy-cannot-reproduce-ci`).
- [ ] 6.2 Cut `cchv-v0.21.0` per Phase 3 — bump, `just sync-version`, `cargo check -q
      -p hub`, both censuses, explicit `git add` (five bump targets now), Guards 1+2,
      commit, tag, push both remotes, publication proof by `${SHA}` with `case` on rc.
      Verify: the proof prints `published on internal` and `published on origin`.
- [ ] 6.3 Phase 4 — `gh run list -R colangelo/claude-code-history-viewer --commit
      $TAGSHA` shows every workflow `success` (Security Audit included: this is the
      first tag after `ac3945ec`, so a red here is new and real); release has 3 assets.
      Verify: the listing and `gh release view`.
- [ ] 6.4 Relay the deploy to infra: hub swap from the release asset **and** distiller
      reinstall from `git cat-file cchv-v0.21.0:scripts/cchv-distill.py`; swap proof
      `GET /v1/healthz .version == "0.21.0"`; both-halves proof after the first tick;
      expected non-200s: none; migration 0009 is an instant `ADD COLUMN`; Gatus must not
      pin the version (widen-before/narrow-after; exact semver if it ever compares).
      Verify: relay sent on a channel that confirms delivery (Channel 0 to
      `infra-peer-relay` while `ac/infra#98` is open), `RELAY_AUDIT`/ack noted.
- [ ] 6.5 Verify live on m4m after infra's ack: `/v1/healthz` version, then after the
      next tick `/v1/healthz/journal` identity triple; record the readings on #39 and
      #40 and close both with the evidence. Verify: the issues are closed with the
      probe output quoted.
