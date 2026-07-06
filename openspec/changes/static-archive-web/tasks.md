# Tasks: static-archive-web

## 1. Hub static hosting (issue #10)

- [x] 1.1 Add `fs` feature to `tower-http` in `crates/hub/Cargo.toml`
- [x] 1.2 Add `static_dir: Option<PathBuf>` to hub config (`static_dir` TOML
      key + `HUB_STATIC_DIR` env, same precedence as existing settings)
- [x] 1.3 Wire `fallback_service(ServeDir)` into `router()` when
      `static_dir` is set; unchanged 404 fallback when unset
- [x] 1.4 Rust tests: `/` serves `index.html` + asset content-type with
      `static_dir` set; `/v1/healthz` wins over a `v1/` dir in static root;
      root 404 without `static_dir`; static root needs no Authorization
- [x] 1.5 `cargo test -- --test-threads=1`, `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all -- --check` green

## 2. Connect gate component (issue #9)

- [x] 2.1 Add i18n keys for the connect screen (title, url/token fields,
      connect button, probe errors, disconnect, localStorage hint) to the
      `archive` namespace in all 5 locales; `pnpm run generate:i18n-types`;
      `pnpm run i18n:validate`
- [x] 2.2 Implement `ConnectGate` (`src/components/ArchiveBrowser/ConnectGate.tsx`):
      form state, probe via cheapest `hubApi` call, versioned localStorage
      persistence (`cchv.archiveWeb.hubConfig`, try/catch), disconnect,
      renders `ArchiveBrowser` when connected; a11y per house checklist
      (labels ↔ inputs via `React.useId`, aria-labels on icon buttons)
- [x] 2.3 Vitest for ConnectGate: first visit shows form; success persists +
      mounts browser; failure shows error, persists nothing; returning visit
      skips form; disconnect clears storage

## 3. Static entry + build target (issue #9)

- [x] 3.1 Add `archive.html` + `src/archive-main.tsx` (i18n import, theme
      fallback, mount ConnectGate) — no store/Tauri/api-adapter imports
- [x] 3.2 Add `vite.archive.config.ts` (input `archive.html`, outDir
      `dist-archive`, reuse base plugins/aliases/manualChunks); ensure output
      lands as `dist-archive/index.html`
- [x] 3.3 Add justfile recipe `archive-web-build` (and a `just
      archive-web-preview` convenience using `vite preview`)
- [x] 3.4 Verify the bundle: build, grep output for `@tauri-apps` (must be
      absent), serve `dist-archive/` statically and smoke-check it loads and
      calls only `/v1/*` on the configured hub
- [x] 3.5 Ensure the default app build is untouched: `pnpm build` still emits
      today's `dist/` and `pnpm tsc --build .` passes

## 4. Quality gate + integration

- [x] 4.1 Full frontend gate: `pnpm tsc --build .`, `pnpm vitest run`,
      `pnpm lint`, `pnpm run i18n:validate`
- [x] 4.2 End-to-end against the live hub on m4m: `HUB_STATIC_DIR=…` hub
      serves `dist-archive/` at `/` while `/v1/healthz` stays JSON; connect
      screen → browse a real archived session same-origin
- [x] 4.3 Update docs: CLAUDE.md CLI/deploy notes + `docs/archive/deployment.md`
      section for `HUB_STATIC_DIR`
- [x] 4.4 Granular commits; close Gitea issues #9/#10 with conclusions; relay
      home-network that the static bundle + `HUB_STATIC_DIR` are available
