# Tasks — hub fail-fast on a rotated Postgres credential

Test-first throughout: `is_auth_code` and the strike transition are the entire safety
property, so they get tests before implementation. Run Rust tests with
`--test-threads=1` (house rule — settings tests mutate `HOME` process-globally).

## 1. Classifier and strike logic (pure, no database)

- [x] 1.1 Create `crates/hub/src/db_watchdog.rs` and declare `pub mod db_watchdog;` in `crates/hub/src/lib.rs`
- [x] 1.2 Write failing unit tests for `is_auth_code`: `Some("28P01")` → true; `Some("53300")`, `Some("57P03")`, `Some("42P01")`, `Some("")`, `None` → false
- [x] 1.3 Implement `is_auth_code(code: Option<&str>) -> bool` so 1.2 passes
- [x] 1.4 Write failing unit tests for the strike transition: an auth failure increments; success resets to 0; a non-auth failure resets to 0; an auth failure after a reset starts from 1; the limit fires at exactly 3 and not at 2
- [x] 1.5 Implement the strike transition (a small pure function or `#[derive(Default)]` struct) so 1.4 passes
- [x] 1.6 Add the `sqlx::Error` → code adapter (`is_auth_failure(&sqlx::Error) -> bool`) delegating to `is_auth_code`, keeping it to a single `matches!` so no logic hides in the untestable layer
- [x] 1.7 `cargo test -p hub --lib -- --test-threads=1` green

## 2. Probe

- [x] 2.1 Implement `probe(database_url: &str) -> Result<(), sqlx::Error>`: establish a **new** connection (not from the pool), run `SELECT 1`, close it
- [x] 2.2 Wrap the probe in a 5 s timeout; on elapsed, return/classify as a non-authentication failure so it resets strikes (spec: a hung attempt must not stall the loop)
- [x] 2.3 Confirm by inspection that neither the probe nor any log statement can emit `database_url` or the password (spec requirement)

## 3. Watchdog loop

- [x] 3.1 Implement `pub async fn run_watchdog(database_url: String, fatal: Arc<Notify>, interval: Duration, limit: u32)`: loop probe → classify → update strikes → sleep
- [x] 3.2 On reaching the limit, log an error naming a rotated/invalid credential as the cause, then `fatal.notify_one()` and return
- [x] 3.3 Log each authentication failure at `warn` with the current strike count and the limit, so the 90 s window is visible in `/tmp/cchv-hub.err` after the fact
- [x] 3.4 Log non-authentication probe failures at `debug` only — a pg1 outage must not spam the hub log for hours

## 4. Wire into `run()`

- [x] 4.1 In `hub::run()`, create `let db_fatal = Arc::new(Notify::new())` and spawn `run_watchdog` with the config's database URL, 30 s interval, limit 3
- [x] 4.2 Replace the bare `axum::serve(listener, app).await?` with a `tokio::select!` over the server and `db_fatal.notified()`
- [x] 4.3 Make the notified arm return an error (`anyhow::bail!`) whose message identifies a sustained authentication failure, so `main()` exits non-zero
- [x] 4.4 Verify `/v1/healthz` code is untouched (spec: health reporting unchanged)

## 5. Integration test pinning the `28P01` assumption

- [x] 5.1 Add `crates/hub/tests/db_auth_classification_test.rs` following the existing `TEST_DATABASE_URL`/`DATABASE_URL` convention in that directory
- [x] 5.2 Derive a deliberately wrong-password URL from the test URL, attempt a connection, and assert the resulting `sqlx::Error` classifies as an authentication failure — this is what fails CI if a future sqlx changes the error mapping
- [x] 5.3 Assert the negative direction too: a connection to a non-routable host or closed port classifies as **not** an authentication failure
- [x] 5.4 `cargo test -p hub -- --test-threads=1` green against a real Postgres

## 6. Quality gates

- [x] 6.1 `cargo fmt --all -- --check`
- [x] 6.2 `cargo clippy --all-targets --all-features -- -D warnings`
- [x] 6.3 `cargo test -p hub -- --test-threads=1`
- [x] 6.4 Confirm the frontend gates are unaffected (no TS/i18n surface in this change)

## 7. End-to-end acceptance before deploy

- [x] 7.1 Point a locally built hub at a scratch/dev Postgres and confirm it starts and serves
- [x] 7.2 `ALTER ROLE … PASSWORD …` underneath the running hub, then confirm it exits within ~90 s with a non-zero status and the expected log line
- [x] 7.3 Confirm the negative case: stop that Postgres entirely and confirm the hub stays up (strikes reset, no exit) — the #17 guarantee
- [x] 7.4 Restart with the new password and confirm it serves again

## 8. Land and ship

- [x] 8.1 Update `docs/archive/deployment.md` §3b: rotation pickup is now automatic, and note the ~90 s detection window
- [x] 8.2 Merge `feature/hub-db-auth-failfast` to `main` (rebase first — `main` sees concurrent pushes) and push to `internal` + `origin`
- [x] 8.3 Bump the version, `just sync-version`, commit, tag `cchv-vX.Y.Z`, push tag to both remotes
- [x] 8.4 Confirm CI published the release with the hub binary asset + `.sha256`
- [x] 8.5 Relay the §2b hub binary swap to infra as **two messages** (home-network #34): swap instructions first, then the verification ask
- [ ] 8.6 After infra confirms: verify `/v1/healthz` is ok and the launch log still shows `db password from bao static-creds/cchv-svc`
- [ ] 8.7 Close cchv Gitea #25 with the commit refs and the deployed version
- [ ] 8.8 Archive this OpenSpec change (`openspec archive`), which promotes `hub-credential-resilience` into `openspec/specs/`
