# Design: static-archive-web

## Context

`ArchiveBrowser` (`src/components/ArchiveBrowser/index.tsx`) is a
self-contained hub client: it takes `config: HubConfig` as a prop and calls
the hub exclusively through `src/services/hubApi.ts` (plain `fetch`). The hub
(`crates/hub`) already sends permissive CORS (`allow_origin(Any)`,
`Authorization` allowed, `X-Total-Count` exposed) precisely so browsers can
call it cross-origin. The desktop/WebUI app wires the config from the Zustand
store; that wiring — not the browser component — is what drags in the Tauri
IPC surface. Verified: the render tree's only store reference is a
**type-only** import (`SearchFilterType` in `ClaudeContentArrayRenderer.tsx`),
erased at compile time.

The hub has no CLI args; configuration is `HUB_CONFIG` TOML or env vars
(`crates/hub/src/config.rs`). `tower-http` is already a hub dependency
(`cors` feature only).

## Goals / Non-Goals

**Goals:**

- A `dist-archive/` static bundle: connect screen → `ArchiveBrowser`, zero
  backend of its own (issue #9).
- Hub optionally serves that bundle at `/` for same-origin, one-process
  deployment (issue #10).
- All quality gates pass: tsc, vitest, lint, i18n:validate, cargo
  test/clippy/fmt.

**Non-Goals:**

- Local-history browsing in the static build (requires a filesystem backend).
- Deploy/launchd/proxy changes (infra's side — relay once shipped).
- server-release CI packaging of `dist-archive` (follow-up if wanted).
- Auth for static assets (tailnet exposure; `/v1` bearer auth unchanged).

## Decisions

1. **Second Vite config (`vite.archive.config.ts`), not a multi-input build.**
   The Tauri/WebUI build must keep emitting exactly today's `dist/`;
   `tauri.conf.json` and the `webui-server` rust-embed both point at it. A
   sibling config with `build.rollupOptions.input = archive.html`,
   `outDir: dist-archive`, reusing the same plugins/aliases (import and spread
   the base config where practical), is simpler and safer than teaching one
   config two output modes via env flags. `archive.html` at the repo root is
   renamed to `index.html` in the output via Vite's input naming (or
   `emptyOutDir` + a tiny rename in the just recipe if Rollup naming fights
   back — recipe owns the invariant either way).

2. **Entry boots only what it needs.** `src/archive-main.tsx`: import
   `./i18n`, theme bootstrapping (reuse the existing dark/light mechanism if
   importable without the store; otherwise `prefers-color-scheme` on
   `document.documentElement`), mount `<ArchiveWebApp/>`. No `AppLayout`, no
   `useAppStore` bootstrap, no updater hooks.

3. **Connect screen as a small new component**
   (`src/components/ArchiveBrowser/ConnectGate.tsx`): owns `HubConfig` state,
   `localStorage` persistence (key `cchv.archiveWeb.hubConfig`, versioned
   shape `{v:1,url,token}`), probe via `hubApi.listProjects(config, {limit:1})`
   or the cheapest existing call, renders `ArchiveBrowser config=…` once
   connected, plus a disconnect button that clears storage. All storage access
   in try/catch (house rule). New i18n keys go in the existing `archive`
   namespace, all 5 locales, then `generate:i18n-types`.

4. **Hub static serving via router fallback.** Add optional
   `static_dir: Option<PathBuf>` to hub config (TOML key `static_dir`, env
   `HUB_STATIC_DIR`). In `router()`, when set:
   `router.fallback_service(ServeDir::new(dir))` (tower-http `fs` feature).
   Explicit `.route("/v1/…")` registrations always win over the fallback, so
   API precedence holds structurally — no path-matching code. When unset,
   behavior is byte-identical to today (fallback stays the default 404).
   Alternative rejected: rust-embed baking `dist-archive` into the hub binary
   — couples the hub build to the frontend toolchain and bloats CI; a runtime
   directory is what infra's launchd job wants anyway.

5. **Testing.** Vitest: ConnectGate (first-visit renders form, successful
   probe persists + mounts browser, failed probe shows error and persists
   nothing, returning visit skips form, disconnect clears). Reuse the
   existing `ArchiveBrowser.test.tsx` fetch-mock patterns. Rust: extend hub
   tests — with `static_dir` set serve `index.html` at `/` and asset with
   content-type; `/v1/healthz` still JSON even with a `v1/` dir present;
   without `static_dir` root stays 404. Follow existing hub test harness
   (`crates/hub/tests/`), single-threaded env-var caveats as in repo rules.

## Risks / Trade-offs

- [Bundle drags in an unexpected Tauri/store module] → the entry imports only
  `ArchiveBrowser` + i18n; verify with a build and grep the bundle for
  `@tauri-apps`; CI-independent `just archive-web-build` keeps it observable.
- [Rollup emits `archive.html` instead of `index.html`] → rename step in the
  just recipe; spec only requires an `index.html` in `dist-archive/`.
- [Token in `localStorage`] → accepted for tailnet-only use; documented in
  the connect screen hint text; disconnect clears it.
- [ServeDir fallback interacting with hub auth middleware] → auth is applied
  per-route today (not a global layer), so the fallback is naturally
  unauthenticated; the precedence test pins this.
- [i18n locale chunks inflate the static bundle] → same manualChunks
  splitting applies; acceptable, lazy-loaded per language.

## Migration Plan

Pure addition — no data, schema, or API changes. Rollback = don't set
`HUB_STATIC_DIR` (hub identical to today) and ignore `dist-archive/`.
After merge: relay home-network to point the m4m hub job at the built bundle
(references issues #9/#10).
