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

## Eval file naming (BOTH tiers — hard requirement)

The loop executes **only** the exact templated filenames: ALL T1 tests go in
the single file `crates/loop-evals/tests/<runId>.eval.test.tsx` and ALL T2
tests in the single file `crates/loop-evals/tests/<runId>_eval.rs` (one
`#[test]`/`it()` per criterion inside). Never split evals across additional
files or invent other names — files with any other name are silently ignored
by the runner and the red check will report every eval as vacuous.

## T1 evals (vitest)

- Live in `crates/loop-evals/tests/` (shared with the T2 tier — the loop
  requires one evals dir), named `<runId>.eval.test.tsx`, picked up via a
  dedicated include glob in `vite.config.ts`. Vitest 4, `jsdom` environment,
  `globals: true`, setup file `src/test/setup.ts` (pre-mocks
  `window.__TAURI__`, `localStorage`, `matchMedia`, `IntersectionObserver`,
  `ResizeObserver` — don't re-mock those). Import app code via the `@/` alias
  (files here sit outside `src/`, so relative imports won't reach it).
- Match the conventions of the existing tests in `src/test/*.test.tsx`:
  `@testing-library/react` (`render`/`screen`/`fireEvent`), `@testing-library/jest-dom`
  matchers, `vi.mock(...)` for service modules / Tauri invoke. Good models:
  `src/test/SessionItem.test.tsx` (component), `src/test/metadataSlice.test.ts`
  (store slice logic).
- **Evals must be self-contained.** During the gate stage no server is running: never
  fetch `http://127.0.0.1:3727`, never spawn the app or the binary. Test components,
  store slices, and utils directly with mocked backends, the way existing tests do.
- Never use `.skip` / `.todo` / `.fails` — a skipped eval counts as vacuous.
- `pnpm lint` and `tsc --build` do **not** cover the evals dir (eslint runs over
  `src/` only) — write lint-clean, type-correct code anyway; vitest transpiles
  without type-checking.

## T2 evals (Rust integration tests)

- Live in `crates/loop-evals/tests/`, named `<runId>_eval.rs`. The
  **`loop-evals` umbrella crate** dev-depends on every workspace crate (`hub`,
  `history-core`, `archive-protocol`) plus `axum`/`tokio`/`reqwest`/`sqlx`/
  `serde_json`/`uuid`/`tempfile`, so evals can exercise hub HTTP endpoints and
  parsers alike. Cargo auto-discovers each file as an integration-test target;
  the loop runs it via `cargo nextest --profile loop` (JUnit output,
  single-threaded — configured in `.config/nextest.toml`).
- **Use T2 for backend-observable acceptance criteria** (hub endpoints,
  provider detection, scanning, session/message parsing, `ProviderId`
  behavior) and T1 for frontend-observable ones. Every criterion needs an
  executable eval in one of the two tiers — there is no manual/rubric tier.
- **Hub-endpoint evals**: spawn an in-process hub exactly like
  `crates/hub/tests/ingest_test.rs::spawn()` does — connect a `PgPool` to
  `TEST_DATABASE_URL` (set by the tier's run command), run `hub::MIGRATOR`,
  build `hub::AppState` with a random `machine_id`/token, serve `hub::router`
  on `127.0.0.1:0`, then drive it with `reqwest`. Seed data through
  `POST /v1/ingest` (the public surface), not raw SQL. A request to a
  **route that doesn't exist yet fails at runtime with 404/405** — that is the
  correct pre-implementation failure shape for new-endpoint criteria and
  compiles fine against the unmodified crate.
- **Evals must COMPILE against the unmodified crate** — this is the Rust
  equivalent of "must fail on the unmodified app". Never reference symbols the
  feature will introduce (a new enum variant, a new module): that is a compile
  error, which the tier reports as one coarse `eval_build` error instead of
  per-criterion failures. Drive new functionality through the *dynamic* surface
  that exists today and assert runtime outcomes:
  - `ProviderId::parse("<id>")` returning `Some`, round-tripping via
    `.as_str()`/`.display_name()` (never `matches!(… ::NewVariant)`).
  - the registry dispatch `providers::load_sessions(id, path, …)` /
    `providers::load_messages(id, path)` with the parsed id.
  - `providers::scan_all_projects()` with `std::env::set_var("HOME", …)`
    pointed at a fixture store in a `TempDir` (established repo pattern; the
    loop profile is single-threaded so this is safe), then filter the result
    by `project.provider`.
- Test against the public API (`history_core::providers::...`). Build fixture
  session stores in a `tempfile::TempDir` inside the test (create the provider's
  directory layout and write fixture files), never against the host's real home
  directory. Model fixtures on the provider unit tests inside
  `crates/history-core/src/providers/*.rs` (e.g. `claude.rs`, `opencode.rs`).
- Providers whose default store location is the user's home should expose (or
  gain) a base-dir-parameterized scan function (see
  `continue_dev::scan_projects_in`) so evals can point them at fixtures.
- Same lint bar as all Rust code: clippy pedantic `-D warnings` + rustfmt.

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
