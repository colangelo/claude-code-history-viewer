set dotenv-load
set windows-powershell := true

# Put pnpm and mise tools to PATH
export PATH := env_var('HOME') + '/.cargo/bin' + PATH_VAR_SEP + justfile_directory() + '/node_modules/.bin' + PATH_VAR_SEP + justfile_directory() + '/.mise/shims' + PATH_VAR_SEP + env_var('PATH')

# Uncomment these if you do not already use mise, if you do not have it configured in your shell or $PATH.
# This will run an isolated, local mise environment, exclusive to this project.

# export MISE_CONFIG_DIR := justfile_directory() + '/.mise'
# export MISE_DATA_DIR := justfile_directory() + '/.mise'

@default:
  just --list --unsorted

# setup build environment
setup: _pre-setup && _post-setup
    # install devtools
    mise install || true

    pnpm install

# OS-specific setup
[windows]
_pre-setup:
    #!powershell -nop
    winget install mise
[linux]
_pre-setup:
[macos]
_pre-setup:

[windows]
_post-setup:
[linux]
_post-setup:
[macos]
_post-setup:
    # Add required Rust targets for universal macOS builds
    rustup target add x86_64-apple-darwin 2>/dev/null || true
    rustup target add aarch64-apple-darwin 2>/dev/null || true

# Run live-reload dev server
dev:
    tauri dev

# Run vite dev server (will not work without tauri, do not run directly)
vite-dev:
    vite

lint:
    eslint .

# Preview production build
vite-preview:
    vite preview

# Build frontend
[windows]
frontend-build: sync-version
    pnpm exec tsc --build
    pnpm exec vite build
[unix]
frontend-build: sync-version
    pnpm exec tsc --build .
    pnpm exec vite build

[windows]
tauri-build:
    tauri build
[linux]
tauri-build:
    tauri build
[macos]
tauri-build:
    tauri build --target universal-apple-darwin

# Copy version number from package.json to Cargo.toml
sync-version:
    node scripts/sync-version.cjs

# Run Tauri CLI
tauri *ARGS:
    tauri {{ARGS}}

test:
    vitest

# Run tests once with verbose output
test-run:
    vitest run --reporter=verbose

# Run tests with UI
test-ui:
    vitest --ui

# Run simple tests only
test-simple:
    vitest run simple

# Setup work environment for a GitHub issue
issue ISSUE_NUMBER:
    ./scripts/setup-issue-work.sh {{ISSUE_NUMBER}}

# List open issues
issues:
    gh issue list --state open --limit 20

# ===== Static Archive Webapp =====

# Build the standalone static archive browser (hub-only, no backend) -> dist-archive/
archive-web-build:
    pnpm exec vite build --config vite.archive.config.ts
    mv dist-archive/archive.html dist-archive/index.html

# Preview the built static archive webapp locally
archive-web-preview: archive-web-build
    pnpm exec vite preview --config vite.archive.config.ts

# ===== WebUI Server Mode =====

# Build server binary with embedded frontend (single binary)
serve-build: frontend-build
    cd src-tauri && cargo build --release --features webui-server

# Build and run server (full rebuild)
# NB: workspace target dir is the repo root, NOT src-tauri/target — and no
# leading `-`, or a missing binary exits 0 and looks like a clean run.
serve-build-run: serve-build
    ./target/release/claude-code-history-viewer --serve

# Run the already-built server binary (no rebuild, instant start)
serve *ARGS:
    ./target/release/claude-code-history-viewer --serve {{ARGS}}

# Run server in development mode (external dist/ for hot reload)
serve-dev: frontend-build
    cd src-tauri && cargo run --features webui-server -- --serve --dist ../dist

# ===== Rust Testing Commands =====

# Run Rust tests with cargo test
# Run Rust tests (single-threaded due to env::set_var("HOME") in tests)
rust-test:
    cd src-tauri && cargo test -- --test-threads=1

# The ARCHIVE crates (history-core, protocol, hub, sync-daemon) — the exact scope,
# flags and thread count `archive-tests.yml` uses, so a local green means the same
# thing CI's green does.
#
# `--test-threads=1` is not optional: these share one database, and under default
# parallelism embed_sweep_test and others fail on each other's rows.
#
# TEST_DATABASE_URL defaults to a LOCAL scratch database. It is never pg1 — point
# it at the live archive and the tests will migrate and write to it.
#
# If a journal_day_test refuses to assert ("the page was truncated ... Refusing to
# assert"), the scratch DB has simply accumulated too many rows across runs — that
# is the test correctly declining to make a claim it cannot support, not a
# regression. Run `just archive-test-reset` and re-run. CI never sees it because
# each run gets a fresh container.
#
# Test the archive crates exactly as CI does (shared DB, so --test-threads=1)
archive-test *ARGS:
    TEST_DATABASE_URL="${TEST_DATABASE_URL:-postgres://$USER@127.0.0.1:5432/cchv_archive_test}" \
    cargo test -p history-core -p archive-protocol -p hub -p sync-daemon {{ARGS}} -- --test-threads=1

# CI's clippy scope for the same crates.
archive-lint:
    cargo clippy -p history-core -p archive-protocol -p hub -p sync-daemon \
        --all-targets --all-features -- -D warnings

# Drop and recreate the LOCAL scratch test database. Loopback only, by construction.
archive-test-reset:
    psql -h 127.0.0.1 -d postgres -X -q \
        -c "DROP DATABASE IF EXISTS cchv_archive_test;" \
        -c "CREATE DATABASE cchv_archive_test OWNER $USER;"
    @echo "cchv_archive_test recreated (migrations run on the next test spawn)"

# Repair historical journal entries in bounded batches (see docs/2026-08-21-journal-day-bucketing.md)
journal-backfill FROM="2026-07-04" BATCH="50" MAX="12":
    scripts/journal-backfill.sh {{FROM}} {{BATCH}} {{MAX}}

# Run the journal distiller's tests (offline; no hub, no LLM)
distill-test:
    uv run --with pytest --with requests pytest scripts/test_cchv_distill.py -q

# Run Rust tests with nextest (faster, parallel)
rust-nextest:
    cd src-tauri && cargo nextest run

# Run Rust tests with coverage
rust-coverage:
    cd src-tauri && cargo llvm-cov nextest --html

# Open Rust coverage report
rust-coverage-open:
    cd src-tauri && cargo llvm-cov nextest --html --open

# Run Rust tests in CI profile
rust-test-ci:
    cd src-tauri && cargo nextest run --profile ci

# Run Rust clippy lints
rust-lint:
    cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings

# Check Rust formatting
rust-fmt-check:
    cd src-tauri && cargo fmt --all -- --check

# Format Rust code
rust-fmt:
    cd src-tauri && cargo fmt --all

# Run Rust benchmarks
rust-bench:
    cd src-tauri && cargo bench

# Run Rust security audit
rust-audit:
    cd src-tauri && cargo audit

# Run all Rust checks (lint, format, test)
rust-check-all: rust-fmt-check rust-lint rust-test

# Watch and run Rust tests on changes
rust-watch:
    cd src-tauri && cargo watch -x test

# Generate Rust documentation
rust-doc:
    cd src-tauri && cargo doc --no-deps --document-private-items --open

# Run property-based tests only
rust-proptest:
    cd src-tauri && cargo test proptest

# Review snapshot changes (insta)
rust-snapshot-review:
    cd src-tauri && cargo insta review

# Install Rust testing tools
rust-tools-install:
    cargo install cargo-nextest --locked
    cargo install cargo-llvm-cov --locked
    cargo install cargo-watch --locked
    cargo install cargo-audit --locked
    cargo install cargo-insta --locked
    cargo install cargo-mutants --locked

# ===== History Archive =====

# Recover expired history from Time Machine backups into the hub archive
# (see docs/archive/timemachine-backfill.md). Ex: just tm-backfill --list
tm-backfill *ARGS:
    ./scripts/tm-backfill.sh {{ARGS}}
