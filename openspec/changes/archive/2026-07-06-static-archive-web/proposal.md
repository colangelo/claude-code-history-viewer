# Proposal: static-archive-web

## Why

The hub archive currently has exactly two frontends: the desktop app and the
WebUI server — both require a running cchv process with filesystem access.
But the Archive mode is already backend-free by construction (`ArchiveBrowser`
talks to the hub straight from the browser via `hubApi.ts`, and the hub ships
permissive CORS). Extracting it into a static web build gives a
zero-process archive UI that any static host — or the hub itself — can serve.
Gitea issues #9 (frontend) and #10 (hub static serving).

## What Changes

- New Vite entry point (`archive.html` + `src/archive-main.tsx`) that boots
  i18n + theme and mounts `ArchiveBrowser` behind a small connect screen
  (hub URL + token, persisted in `localStorage`) — no Tauri, no `/api`
  adapter, no Zustand app-store bootstrap.
- New build target producing `dist-archive/` (separate from the Tauri/WebUI
  `dist/`), wired into the justfile.
- New i18n keys for the connect screen in all 5 locales (en, ko, ja, zh-CN,
  zh-TW) + regenerated types.
- Hub: new optional `--static-dir` flag / `HUB_STATIC_DIR` env /
  `static_dir` config key; when set, the hub serves that directory at `/`
  (currently 404) via `tower-http::services::ServeDir`, with `/v1/*` API
  routes keeping priority. Same-origin deployment eliminates CORS and
  mixed-content concerns entirely.
- Tests: vitest for connect/persistence logic and the static entry mount;
  Rust tests for static serving + `/v1` precedence.

## Capabilities

### New Capabilities

- `static-archive-webapp`: standalone static web build of the hub archive
  browser — connect screen, persisted hub config, browse/search UI, built as
  plain static files with no backend of its own.
- `hub-static-hosting`: the hub optionally serves a static directory at `/`
  without shadowing the `/v1` API.

### Modified Capabilities

<!-- none — archive-search-api requirements are unchanged; the webapp is a
     pure consumer of the existing API -->

## Impact

- Frontend: new `archive.html`, `src/archive-main.tsx`, small connect
  component; `vite.config.ts` or a sibling config gains a second build
  target; `src/i18n/locales/*/archive.json` (or a new namespace) gains keys;
  `src/i18n/types.generated.ts` regenerated. `ArchiveBrowser` itself should
  need no changes — transitive store imports in reused renderers are the main
  risk and may require a light shim.
- Backend: `crates/hub/src/config.rs` (new optional setting),
  `crates/hub/src/lib.rs` (router fallback service), `crates/hub/Cargo.toml`
  (`tower-http` `fs` feature).
- Build/CI: justfile recipes (`archive-web-build`); optionally attach
  `dist-archive` to the server-release workflow later (not required here).
- Deployment: infra can point the existing m4m hub launchd job at
  `dist-archive/` (follow-up relay once shipped; infra is separately hosting
  the full WebUI per the 2026-07-06 relay).
