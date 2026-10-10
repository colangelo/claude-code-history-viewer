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

- [x] 2.1 Delete the updater: `commands/update.rs`, its registration and the updater
      plugin config.
- [x] 2.2 Convert the 79 `#[tauri::command]` functions to plain functions. `State<X>` →
      `&X`; delete `#[cfg(feature = "webui-server")]` twins so one function remains per
      command.
- [x] 2.3 Delete `run_tauri()`, `tauri.conf.json`, `capabilities/`, `build.rs`'s
      `tauri_build`, `main.rs`'s windows-subsystem attribute, and every `tauri*` dependency
      in `Cargo.toml`.
- [x] 2.4 Replace `tauri-plugin-log` with the logging `--serve` needs; `--export` stays
      quiet on stdout.
      **Nothing to replace:** the plugin was a locked dependency but never registered, so
      `--serve` had no `log` backend before the cut either. Its output is `eprintln!` and
      stays so; adding a logger is a separate decision.
- [x] 2.5 D3: make `webui-server` the default feature (or fold it in, per Q3).
- [x] 2.6 Remove the watcher's `AppHandle` emit path; keep the web stubs as they are.
- [x] 2.7 Delete the `--session` startup flag (`cli.rs`, `get_startup_session_hint`,
      `src/lib/preloadSession.ts`, the `App.tsx` listener). It only preloads a session in
      the GUI window and never worked in the WebUI (the archive webapp's hash deep links
      are the browser-side equivalent). The server route
      added in `638b7bea` goes with it.
- [x] 2.8 `cargo tree -i tauri --target all` prints nothing; clippy (CI's Rust version)
      `--all-targets --all-features -D warnings`; `cargo test -- --test-threads=1`.

## 3. Frontend

- [x] 3.1 Remove each Tauri branch from 1.3, keeping the web half; resolve each "missing"
      row as decided.
- [x] 3.2 Delete `useGitHubUpdater.ts`, `useSmartUpdater.ts` and the update UI; remove
      the `update` namespace from all 5 locales; regenerate i18n types.
- [x] 3.3 Drop `@tauri-apps/*` and the `tauri` scripts from `package.json`;
      `pnpm install`.
- [x] 3.4 `pnpm tsc --build .`, `pnpm vitest run`, `pnpm lint`,
      `pnpm run i18n:validate`, `just archive-web-build` (the archive webapp must be
      byte-for-byte unaffected apart from removed dead code).

## 4. CI, docs, verification

- [x] 4.1 `rust-tests.yml`: drop the GTK/WebKit install steps. Delete
      `update-flow-tests.yml`.
- [x] 4.2 `security-audit.yml` absence guard: drop `rkyv`. Hand the `.cargo/audit.toml`
      edit to an attended session (permission-gated).
      The cut also drops `quick-xml` and `plist` from the lock, so the edit removes three
      entries (0194, 0195, 0235). Checked: `cargo audit` passes on the new lock with them
      gone, and fails (3 vulnerabilities) on `main`'s lock with the same config, so the
      edit must land with or after this change, never before.
- [x] 4.3 Re-run 1.2's exports and compare: byte-identical HTML and JSON (`cmp -s`).
      1.2's goldens were gone from `/private/tmp` by 2026-10-10, so both binaries were
      rebuilt (`main` 260ed420 and this branch) and run on the same three frozen Claude
      sessions (14 KB / 744 KB / 50 MB, picked by size): all 6 outputs identical; a
      repeat export was identical too, so the comparison is deterministic.
- [x] 4.4 `--serve --no-auth` smoke test: load the WebUI, browse a project and a session,
      and confirm no `/api/*` 404/405 in the server log.
      Headless form, under a throwaway `HOME`: `GET /` 200, `GET /api/events` (SSE) 200,
      and a POST to each of the 55 command names the frontend calls: 0 answered 404/405
      (control: an unknown command answers 405). No browser session was driven.
- [x] 4.5 Update AGENTS.md (*Desktop app (retired…)*, *What CI builds*, Project Overview
      dev commands), the Justfile (`dev`, `tauri-build`), and `cchv-find` §3 (relay to the
      CONTEXT owner if its commands change).
- [ ] 4.6 Record the "after" numbers from 1.1 in the PR/commit message. CI green on every
      workflow for the final sha.
      **After, 2026-10-10** (m4m, same settings as 1.1, both trees from `cargo clean
      --release`): dependency graph **664 → 237** crates (1.1's method, reproduced exactly
      on `main`); release build compiles **437 → 156** crates in **2m03s → 41s** (cargo's
      own `Finished` time; the after run's wall clock included heavy-lock wait); release
      binary **38,917,088 → 16,904,480 bytes** (−57 %). (1.1's 40,269,912 was an older
      frontend; `main` measured today is the fair "before".) CI green: pending the PR.

## 5. Optional, after group 4 is green (Q2)

- [ ] 5.1 Rename `src-tauri` → `crates/viewer` (or the agreed name) in its own commit:
      path-only, no logic. Update workflows, Justfile, docs and the `cchv-find` skill in
      that same commit.
