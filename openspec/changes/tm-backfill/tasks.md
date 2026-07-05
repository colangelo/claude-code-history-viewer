# Tasks — tm-backfill

## 1. sync-daemon: single-pass mode + hostname override

- [x] 1.1 Add `CCHV_HOSTNAME` override in `Identity::load_or_create` (fall back to `gethostname()` when unset/empty) with unit tests
- [x] 1.2 Add `--once` flag: parse in `main.rs`, add a `run_once_and_exit()` path in `lib.rs` that skips watcher setup, runs one `sync::run_once` pass, logs stats, and returns an error (→ non-zero exit) when `stats.errors > 0`
- [x] 1.3 `cargo test -p sync-daemon -- --test-threads=1`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` (workspace crates, `SQLX_OFFLINE=true`)

## 2. Backfill script

- [x] 2.1 Write `scripts/tm-backfill.sh` (bash, `set -euo pipefail`): flag parsing (`--list`, `--snapshot <stamp>`, `--all`, `--store <path>`, `--machine <label>`, `--user <name>`, `--dry-run`, `--daemon-bin <path>`), hub config read from `~/.config/cchv/daemon.toml`, hub `/v1/healthz` preflight
- [x] 2.2 Snapshot enumeration via `diskutil info -plist` (store → device) + `diskutil apfs listSnapshots`, `--list` output with per-snapshot claude-dir presence and >30-day gap warnings
- [x] 2.3 Mount/stage/ingest/unmount loop: `mount_apfs -o ro -s`, home-dir discovery (`*- Data/Users/<user>` glob with `Users/<user>` fallback), symlink-safe claude-root selection (`.config/claude` real-dir preferred, never follow symlinks), fake-home staging via symlink, per-machine state dir under `~/.config/cchv/backfill/<label>/` (local label seeds machine_id from `~/.claude-history-sync/machine_id`; foreign label sets `CCHV_HOSTNAME`), `cchv-sync-daemon --once` invocation, EXIT trap that unmounts
- [x] 2.4 Verify the script end-to-end against a real snapshot on m4m (re-ingest the proven 2026-04-19 tifo snapshot; expect all-dedup result and 0 errors), plus `--list` and `--dry-run` runs

## 3. Docs + wiring

- [x] 3.1 Write `docs/archive/timemachine-backfill.md` runbook: prerequisites, per-machine usage, foreign-disk recovery (ac-mbp walkthrough), coverage math + m4m's 2026-03-17..19 hole, symlink trap, troubleshooting (sandbox/mount permission, hub unreachable, binary on foreign hosts)
- [x] 3.2 Add Justfile recipe(s) (`just tm-backfill …` passthrough to the script)
- [ ] 3.3 Update `AGENTS.md`/`CLAUDE.md` pointer if the repo keeps an ops index (one line linking the runbook); commit granularly (crate change, script, docs)
