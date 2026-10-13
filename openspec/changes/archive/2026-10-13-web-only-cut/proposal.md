# Proposal

## Why

We retired the desktop distribution months ago, but `src-tauri` still pulls in the full
webview stack for every build, including the `--features webui-server` build that is the
only thing anyone runs (Gitea #23). The cost shows up in three places:

- Every Ubuntu CI job installs GTK and WebKit.
- `cargo audit` has to carry ignores for advisories that sit only in that stack
  (RUSTSEC-2026-0235 via `tauri-plugin-log`).
- There is a whole bug class that exists only because the code has two front doors:
  desktop IPC and axum WebUI routes with no parity between them (#340, #355, #8).

The archive stack doesn't depend on any of it, so the cut is independent of other work.

## What Changes

- **BREAKING (local only):** the Tauri desktop GUI no longer builds. `src-tauri` stops
  depending on `tauri`, `tauri-build` and the ten `tauri-plugin-*` crates. Nothing we ship
  is affected: the hub, sync-daemon, webapp tarball and history-core don't use them.
- The local binary keeps its CLI surface unchanged: `--export` (HTML/JSON) and `--serve`
  (WebUI server). The `cchv-find` skill §3 is the consumer that must keep working.
- The vestigial updater goes: `commands/update.rs`, `useGitHubUpdater.ts`,
  `useSmartUpdater.ts`, the updater plugin config, the `update` i18n namespace (73 keys
  × 5 locales) and `update-flow-tests.yml`.
- The frontend loses its Tauri branches: 41 `isTauri` references, 13 files importing
  `@tauri-apps/*`. `api()` keeps only the HTTP path. Native dialogs, the opener and the
  store plugin fall back to the web behaviour the WebUI already uses.
- CI drops `libgtk-3-dev` and `libwebkit2gtk-4.1-dev` from `rust-tests.yml`, and the
  security-audit ignore for `rkyv` goes away with its only dependency chain.
- The commands move from `#[tauri::command]` + `State<…>`/`AppHandle` to plain functions
  that the axum handlers call directly. The axum surface is then the single front door.

## Capabilities

### New Capabilities

- `local-viewer-cli`: the locally built viewer binary (today `src-tauri`). It exports a
  session headless, serves the WebUI, and builds without any webview stack.

### Modified Capabilities

None. No existing spec covers the desktop shell or the WebUI server. The archive specs
(`static-archive-webapp`, `hub-static-hosting`, …) describe artifacts this change doesn't
touch.

## Impact

- **Code:** `src-tauri/**`: 79 `#[tauri::command]` attributes; about 10 files taking
  Tauri `State`/`AppHandle`/`Emitter`; the desktop builder in `lib.rs::run_tauri`;
  `tauri.conf.json`; `capabilities/`. Frontend `src/**` Tauri branches. `package.json`
  `@tauri-apps/*` deps and the `tauri` scripts.
- **CI:** `rust-tests.yml` (system deps; the Benchmarks job keeps working),
  `security-audit.yml` + `.cargo/audit.toml` (`.cargo/` is permission-gated, so an
  attended session pastes that change), deleting `update-flow-tests.yml`.
- **Docs:** AGENTS.md *Desktop app (retired…)* and *What CI builds* sections, the
  Justfile recipes `dev`/`tauri-build`, and the `cchv-find` skill §3 commands if the
  binary or crate path changes.
- **Upstream sync:** the supply chain is `crates/history-core` (parsers), which this change
  doesn't touch. Porting upstream UI or command changes into `src-tauri` gets harder.
  That's accepted, since we no longer take them.
- **Dependencies:** about 40 lockfile entries are clearly desktop-only (`tauri*`, `wry`,
  `tao`, `webkit2gtk*`, `gtk*`, `gdk*`, `atk*`, `javascriptcore*`, `soup*`). The exact
  number that leaves is measured in task 1.1.
