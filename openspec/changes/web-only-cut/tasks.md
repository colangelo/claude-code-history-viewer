# Tasks

> **Gate:** do not start group 2 until ac has answered design.md Q1–Q3. Group 1 is
> measurement only and safe to run first.

## 1. Baseline (no code change)

- [x] 1.1 Record the "before" numbers: `cargo tree -p claude-code-history-viewer
      --features webui-server --target all --prefix none | sort -u | wc -l`, the
      desktop-only lockfile entries, a clean `cargo build --release --features webui-server`
      wall time (under the m4m heavy-job lock), and the release binary size.
      **Done 2026-10-09** on m4m, rustc 1.98, `--features webui-server`, release:
      dependency graph 664 crates (normal+build edges), **254 with the 12 `tauri*` crates
      pruned**, so ~410 arrive only through Tauri; 40 desktop-only lockfile names; binary
      **40,269,912 bytes**; build 162 s wall under `lockf`+`nice -n 15`+6 jobs with 437
      crates compiled (warm cache, NOT a cold number). 4.6 must rebuild **before and
      after** from a full `cargo clean` with identical settings to compare times.
- [x] 1.2 Capture golden outputs: `--export` of three real sessions (Claude, Codex, one
      other provider) as HTML and JSON, kept under `/private/tmp` (they contain transcript
      text: never commit them).
      **Done 2026-10-09:** three Claude sessions (16 KB, 492 KB, 50 MB of JSONL), HTML +
      JSON each, in `/private/tmp/claude-501/cchv-night/web-only-baseline/golden/`, with
      the inputs copied beside them (retention would otherwise delete them). Control: a
      repeat export of every one was byte-identical, so 4.3's `cmp -s` can actually fail.
      `--export` reads Claude JSONL only, so "Codex / other provider" does not apply.
- [x] 1.3 List every `isTauri()` branch and `@tauri-apps/*` import in `src/` with its web
      half: present, or missing and needs a decision. Done: design.md appendix.

## 2. Backend: remove Tauri from `src-tauri`

- [ ] 2.1 Delete the updater: `commands/update.rs`, its registration and the updater
      plugin config.
- [ ] 2.2 Convert the 79 `#[tauri::command]` functions to plain functions. `State<X>` →
      `&X`; delete `#[cfg(feature = "webui-server")]` twins so one function remains per
      command.
- [ ] 2.3 Delete `run_tauri()`, `tauri.conf.json`, `capabilities/`, `build.rs`'s
      `tauri_build`, `main.rs`'s windows-subsystem attribute, and every `tauri*` dependency
      in `Cargo.toml`.
- [ ] 2.4 Replace `tauri-plugin-log` with the logging `--serve` needs; `--export` stays
      quiet on stdout.
- [ ] 2.5 D3: make `webui-server` the default feature (or fold it in, per Q3).
- [ ] 2.6 Remove the watcher's `AppHandle` emit path; keep the web stubs as they are.
- [ ] 2.7 Delete the `--session` startup flag (`cli.rs`, `get_startup_session_hint`,
      `src/lib/preloadSession.ts`, the `App.tsx` listener). It only preloads a session in
      the GUI window and never worked in the WebUI (the archive webapp's hash deep links
      are the browser-side equivalent). The server route
      added in `638b7bea` goes with it.
- [ ] 2.8 `cargo tree -i tauri --target all` prints nothing; clippy (CI's Rust version)
      `--all-targets --all-features -D warnings`; `cargo test -- --test-threads=1`.

## 3. Frontend

- [ ] 3.1 Remove each Tauri branch from 1.3, keeping the web half; resolve each "missing"
      row as decided.
- [ ] 3.2 Delete `useGitHubUpdater.ts`, `useSmartUpdater.ts` and the update UI; remove
      the `update` namespace from all 5 locales; regenerate i18n types.
- [ ] 3.3 Drop `@tauri-apps/*` and the `tauri` scripts from `package.json`;
      `pnpm install`.
- [ ] 3.4 `pnpm tsc --build .`, `pnpm vitest run`, `pnpm lint`,
      `pnpm run i18n:validate`, `just archive-web-build` (the archive webapp must be
      byte-for-byte unaffected apart from removed dead code).

## 4. CI, docs, verification

- [ ] 4.1 `rust-tests.yml`: drop the GTK/WebKit install steps. Delete
      `update-flow-tests.yml`.
- [ ] 4.2 `security-audit.yml` absence guard: drop `rkyv`. Hand the `.cargo/audit.toml`
      edit to an attended session (permission-gated).
- [ ] 4.3 Re-run 1.2's exports and compare: byte-identical HTML and JSON (`cmp -s`).
- [ ] 4.4 `--serve --no-auth` smoke test: load the WebUI, browse a project and a session,
      and confirm no `/api/*` 404/405 in the server log.
- [ ] 4.5 Update AGENTS.md (*Desktop app (retired…)*, *What CI builds*, Project Overview
      dev commands), the Justfile (`dev`, `tauri-build`), and `cchv-find` §3 (relay to the
      CONTEXT owner if its commands change).
- [ ] 4.6 Record the "after" numbers from 1.1 in the PR/commit message. CI green on every
      workflow for the final sha.

## 5. Optional, after group 4 is green (Q2)

- [ ] 5.1 Rename `src-tauri` → `crates/viewer` (or the agreed name) in its own commit:
      path-only, no logic. Update workflows, Justfile, docs and the `cchv-find` skill in
      that same commit.
