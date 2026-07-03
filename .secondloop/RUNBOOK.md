# RUNBOOK — claude-code-history-viewer

Tauri 2 desktop app (plus an optional headless Axum web server) for browsing Claude
Code conversation history. Frontend: React 19 + TypeScript + Vite 7 + Tailwind 4 in
`src/`. Backend: Rust in `src-tauri/`.

## Architecture pointers

- **Frontend**: components in `src/components/` (shadcn-style primitives in
  `src/components/ui/`), zustand state in `src/store/` (`useAppStore` composed from
  slices in `src/store/slices/`), hooks in `src/hooks/`, pure helpers in `src/utils/`,
  API/service layer in `src/services/`, types in `src/types/`. Path alias `@/` → `src/`.
- **Backend**: Tauri commands in `src-tauri/src/commands/`; the headless WebUI server
  (cargo feature `webui-server`) in `src-tauri/src/server/` (`mod.rs` router,
  `handlers.rs`, `auth.rs`). The web frontend reaches the same command surface over
  HTTP — see `docs/server-guide.md` for the API and CLI flags.
- **i18n**: react-i18next with 5 locales under `src/i18n/locales/` (en, ko, ja, zh-CN,
  zh-TW). Every user-facing string goes through a translation key and must be added to
  **all** locales — `pnpm run i18n:validate` is in the gate and fails on drift. If you
  add keys, regenerate types with `pnpm run generate:i18n-types`
  (`src/i18n/types.generated.ts` is generated; never hand-edit it).
- **Data source**: the app reads conversation history from provider directories under
  the user's home (`~/.claude` and friends). A fresh worktree/server serves whatever
  the host user happens to have — possibly an **empty dataset**. Never write evals or
  verification steps that assume pre-existing history data, and make sure new UI
  handles the empty state gracefully (browser verification may see an empty app).

## T1 evals (vitest)

- Live in `src/test/evals/`, named `<runId>.eval.test.tsx`. Vitest 4, `jsdom`
  environment, `globals: true`, setup file `src/test/setup.ts` (pre-mocks
  `window.__TAURI__`, `localStorage`, `matchMedia`, `IntersectionObserver`,
  `ResizeObserver` — don't re-mock those).
- Match the conventions of the existing tests in `src/test/*.test.tsx`:
  `@testing-library/react` (`render`/`screen`/`fireEvent`), `@testing-library/jest-dom`
  matchers, `vi.mock(...)` for service modules / Tauri invoke. Good models:
  `src/test/SessionItem.test.tsx` (component), `src/test/metadataSlice.test.ts`
  (store slice logic).
- **Evals must be self-contained.** During the gate stage no server is running: never
  fetch `http://127.0.0.1:3727`, never spawn the app or the binary. Test components,
  store slices, and utils directly with mocked backends, the way existing tests do.
- Never use `.skip` / `.todo` / `.fails` — a skipped eval counts as vacuous.
- Evals are linted by the gate (`pnpm lint` runs eslint over `src/`, typescript-eslint
  recommended + react-hooks rules) — keep them lint-clean. `tsc --build` excludes
  `src/test/**`, but write type-correct code anyway; vitest transpiles without checking.

## Gate expectations

- `pnpm lint` covers `src/` only; Rust is gated separately.
- Rust lints are **clippy pedantic with `-D warnings`** (`just rust-lint`) plus a
  rustfmt check. Run `just rust-fmt` and fix all clippy findings before finishing any
  Rust change — pedantic is strict.
- Rust tests run single-threaded (`--test-threads=1`) because some tests set env vars
  (e.g. `HOME`). Don't write Rust tests that require parallelism; reuse the helpers in
  `src-tauri/src/test_utils.rs`. Rust testing guide: `src-tauri/TESTING.md`.
- Versioning: `package.json` is the single source of truth; `scripts/sync-version.cjs`
  (run automatically by `frontend-build`) copies the version into
  `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. Never hand-edit version
  fields in those files.
- Husky/lint-staged commit hooks are bypassed by the loop; the `[gate]` commands are
  the enforced truth.

## What not to touch

- Generated/pipeline files: `src/i18n/types.generated.ts`, `scripts/`,
  `.github/workflows/`, `src-tauri/capabilities/`, `src-tauri/icons/`.
- Security guards: do not weaken the loopback/auth startup checks in
  `src-tauri/src/lib.rs` (`validate_auth_startup_options`,
  `validate_account_cookie_security`) or `src-tauri/src/server/auth.rs`. The server
  intentionally refuses non-loopback binds without auth.

## Serving / browser verification

- The loop serves the headless server on `http://127.0.0.1:3727/` (`--serve --host
  127.0.0.1 --no-auth`, frontend from a fresh `dist/` via `--dist`). Health check:
  `GET /health` returns 200 JSON when ready.
- Remember the empty-dataset caveat above when interpreting browser evidence.

## Docs entry points

- `README.md` — overview and features. `docs/server-guide.md` — headless server API,
  flags, deployment. `src-tauri/TESTING.md` — Rust testing stack. `CLAUDE.md` — repo
  conventions. There is no OKF index; start from `README.md` and `docs/`.
