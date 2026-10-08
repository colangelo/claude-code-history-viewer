# Design

## Context

`src-tauri` is one crate with two front doors onto the same command functions:

- **Desktop:** `lib.rs::run_tauri()` registers 79 `#[tauri::command]` functions with
  `tauri::Builder` and adds ten plugins.
- **WebUI:** `server/mod.rs::build_router` (feature `webui-server`) exposes the same
  functions as axum POST routes in `server/handlers.rs`.

`run()` dispatches `--export`, then `--serve`, then falls through to the desktop
builder. Nobody launches the desktop app; the `--export`/`--serve` CLI is real and used
(`cchv-find` §3, verified 2026-07-26).

Tauri reaches past the attributes in three places, measured 2026-10-09:

1. **Managed state.** `metadata.rs`, `settings.rs`, `mcp_presets.rs`,
   `unified_presets.rs`, `claude_settings.rs`, `archive.rs` and `cli.rs` take
   `tauri::State<…>`. The server already holds its own state (`server/state.rs`
   `AppState { metadata, … }`), and its handlers call parallel non-`State` entry points.
   Some of those already sit behind `#[cfg(feature = "webui-server")]`
   (`claude_settings.rs` ×7, `session/mod.rs`).
2. **Events.** `watcher.rs` emits through `AppHandle`/`Emitter`. The server stubs
   `start_file_watcher`/`stop_file_watcher` ("not available in web mode") and already
   carries a broadcast `event_tx` plus an SSE `/events` route.
3. **Plugins.** dialog, fs, store, opener, os, process, http, log, updater,
   single-instance. Only `log` matters headless; the rest serve the GUI.

## Goals / Non-Goals

**Goals**
- `src-tauri` compiles with no `tauri*` crate in its dependency graph:
  `cargo tree -i tauri --target all` prints nothing.
- `--export` and `--serve` behave byte-for-byte as before on the same inputs.
- One front door: every command reachable from the frontend is an axum route.
- The CI Ubuntu jobs need no GTK/WebKit packages.

**Non-Goals**
- Rewriting the frontend. The WebUI build of `src/` already works over HTTP; this change
  only deletes its Tauri branches.
- Touching `crates/history-core`, hub, sync-daemon or the static archive webapp.
- Restoring a live file watcher in web mode. The SSE plumbing exists but is unused today;
  wiring it up is a separate change.

## Decisions

### D1. Remove Tauri outright (recommended) — not feature-gate it, not split into a new crate

| Option | What | For | Against |
|---|---|---|---|
| **A. Feature-gate** | `tauri` optional behind `desktop`; `#[cfg_attr(feature="desktop", tauri::command)]` on 79 fns | Smallest diff; GUI still buildable | Keeps both front doors and the parity bug class; every command keeps two signatures; CI must build both feature sets to stay honest |
| **B. New crate** | Move CLI + server into `crates/viewer` (or similar), delete `src-tauri` | Cleanest end state | Largest diff in one go; moves files, so every path in CI/Justfile/docs/skill changes at once, along with the logic |
| **C. Remove in place** | Strip Tauri from `src-tauri`, keep the path | Logic change and path change are separable; `cchv-find`, CI and Justfile paths keep working | The directory name is a misnomer until an optional rename |

**Recommendation: C**, with the rename as an optional, mechanical follow-up (task group 5).
A only makes sense if someone still runs the GUI (Q1).

### D2. Commands become plain functions; handlers own the state

Drop `#[tauri::command]`. Where a function takes `tauri::State<X>`, take `&X` instead (the
server's `AppState` already owns the same `MetadataState`). Where a
`#[cfg(feature = "webui-server")]` twin exists, keep one function and delete the twin.
`AppHandle`-only functions (watcher emit, updater) are deleted, not ported; see Non-Goals.

### D3. Keep `webui-server` as the default feature, or fold it in

Without Tauri, the default build has no front door at all except `--export`. Make
`webui-server` the default feature (recommended: least churn, `--no-default-features` still
gives an export-only binary). The alternative is to drop the feature and make axum
unconditional.

### D4. Delete the updater and the `update` i18n namespace

The code is dormant (AGENTS.md has flagged it "safe to remove" since 2026-07-26), and the
namespace's 73 keys exist only for it. `i18n:validate` and `generate:i18n-types` must
pass after removal.

## Risks / Trade-offs

- **A frontend path that only Tauri served.** Native folder picker, opener, store-backed
  settings. *Mitigation:* every `isTauri()` branch is audited (task 3.1). The web branch
  already exists because the WebUI ships today; a branch with no web half is listed and
  decided, not silently dropped.
- **`log` without `tauri-plugin-log`.** The CLI still needs logging. *Mitigation:*
  `env_logger`, or keep what `--serve` already initialises (task 2.4 checks).
- **Divergence from upstream `src-tauri`.** Accepted. Upstream's supply chain into this
  fork is `crates/history-core` only (AGENTS.md *Branch Strategy*).
- **`.cargo/audit.toml` is permission-gated for agents.** The ignore removal is pasted by an
  attended session, as with `2a318594`.

## Open Questions (ac)

- **Q1.** Does anyone still launch the desktop GUI, on any Mac (including ac-mbp)? *If yes →
  Option A instead of C.* Recommendation: no; proceed with C.
- **Q2.** Rename `src-tauri` afterwards (to e.g. `crates/viewer`)? Recommendation: yes,
  but as its own commit after C lands green, so a path break is never mixed with a logic
  break.
- **Q3.** D3: keep `webui-server` as a default feature, or make axum unconditional?
  Recommendation: default feature.

## Appendix: frontend Tauri inventory (task 1.3, measured 2026-10-09)

The 41 `isTauri` references and 13 `@tauri-apps/*` importers fall into two kinds: branches
that already have a web half, and desktop-only affordances that have none. Nothing in the
second kind is a feature the WebUI has today. Removing it removes a button that is already
hidden in the browser.

| Where | Tauri half | Web half today | After the cut |
|---|---|---|---|
| `services/api.ts` | `invoke()` | `fetch('/api/…')` | web half only |
| `services/storage.ts` | `plugin-store` | `localStorage` (`webui:<name>:` prefix) | web half only. Desktop store contents are not migrated (nobody runs desktop, Q1) |
| `utils/fileDialog.ts` (save ×3), `ArchiveBrowser` export | `plugin-dialog` save + `write_text_file` | Blob download | web half only |
| `useLanguageStore.ts` | `plugin-os` locale | `navigator.language` | web half only |
| `utils/platform.ts` `openExternal` | `plugin-opener` | `window.open` | web half only |
| `serverSlice.loadServerConfig` | hard-codes not-read-only | `get_server_config` | web half only |
| `ArchiveBrowser` "open folder", `useSessionEditing` "reveal in Finder" | `revealItemInDir` | **none**: button hidden | delete; a browser can't reveal a path on the server's disk |
| `CustomDirectoriesSection`, `FolderSelector` folder picker | `plugin-dialog` open | **none**: text input only | delete the picker button; typing the path stays |
| `useFileWatcher.ts` | `listen()` on watcher events | **none**: manual refresh | unchanged (Non-Goal: SSE wiring is separate) |
| `App.tsx` `cli-session-hint` listener | single-instance second launch | **none**: already catches and warns | delete together with `--session` (task 2.7) |
| `WslSection.tsx` | Windows + Tauri only | **none**: hidden | delete the section; the server-side WSL scan in `commands/wsl.rs` stays |
| `Header.tsx` traffic-light inset, `PlatformProvider` `desktop` flag | macOS window chrome | n/a | delete |
| `useUpdater.ts`, `SimpleUpdateModal.tsx` | updater plugin | n/a | delete (D4) |
