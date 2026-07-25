//! Pins the one external assumption the credential watchdog rests on
//! (`openspec/changes/hub-db-auth-failfast/`): that a real Postgres rejecting a
//! password surfaces as a `sqlx::Error` carrying SQLSTATE `28P01`, and that an
//! unreachable server does NOT.
//!
//! This matters because the watchdog exits the process on the first condition
//! and must never exit on the second. Both are properties of `sqlx`'s error
//! mapping, not of our code: in 0.8.6 the connect path returns non-transient
//! `Error::Database` directly (only `53300`/`57P03` are retried), which is what
//! makes `28P01` observable rather than collapsed into `PoolTimedOut`. If a
//! future sqlx changes that, this test fails here instead of the hub silently
//! failing to self-heal a month later, at the next credential rotation.
//!
//! Requires `TEST_DATABASE_URL`/`DATABASE_URL`, like the other hub tests.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use hub::db_watchdog::is_auth_failure;
use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, PgConnection};
use tokio::sync::Notify;

const WRONG_PASSWORD: &str = "definitely-not-the-password-9f3a1c";

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

/// Same host, port, user and database as the real test target — only the
/// password differs, so the connection fails for exactly one reason. Built with
/// sqlx's own options parser rather than a URL crate, to avoid adding a
/// dependency for a single test.
fn with_wrong_password(url: &str) -> PgConnectOptions {
    PgConnectOptions::from_str(url)
        .expect("test db url must parse")
        .password(WRONG_PASSWORD)
}

/// The same thing as a URL string, for `run_watchdog`, which takes one. Rebuilt
/// from sqlx's parsed components rather than by string surgery on the original.
fn wrong_password_url(url: &str) -> String {
    let o = PgConnectOptions::from_str(url).expect("test db url must parse");
    format!(
        "postgres://{}:{}@{}:{}/{}",
        o.get_username(),
        WRONG_PASSWORD,
        o.get_host(),
        o.get_port(),
        o.get_database().unwrap_or("postgres"),
    )
}

#[tokio::test]
async fn a_rejected_password_classifies_as_an_auth_failure() {
    let opts = with_wrong_password(&test_db_url());

    // Not every server enforces passwords: a local dev Postgres on `trust`
    // pg_hba accepts any password, so there is no rejection to classify. Detect
    // that instead of failing for the wrong reason — and say so loudly, because
    // a test that quietly no-ops everywhere is worse than no test. CI runs
    // `postgres:16` with POSTGRES_PASSWORD over TCP (scram-sha-256), which is
    // where this assertion actually binds.
    match PgConnection::connect_with(&opts).await {
        Err(err) => assert!(
            is_auth_failure(&err),
            "a real password rejection must classify as an auth failure, otherwise the \
             hub will never self-heal a rotated credential; got: {err:?}"
        ),
        Ok(conn) => {
            let _ = conn.close().await;
            eprintln!(
                "SKIPPED a_rejected_password_classifies_as_an_auth_failure: this server \
                 accepted a deliberately wrong password, so its pg_hba is `trust` and it \
                 cannot exercise SQLSTATE 28P01. Run against a password-enforcing server \
                 (CI does) for this assertion to mean anything."
            );
        }
    }
}

#[tokio::test]
async fn an_unreachable_server_does_not_classify_as_an_auth_failure() {
    // Port 1 on loopback: nothing listens, so this fails before any handshake.
    let err = PgConnection::connect("postgres://someone:something@127.0.0.1:1/nothing")
        .await
        .expect_err("connecting to a closed port must fail");

    assert!(
        !is_auth_failure(&err),
        "an unreachable server must NOT classify as an auth failure — that is what \
         keeps a pg1 outage or DNS flake from exiting the hub (issue #17); got: {err:?}"
    );
}

#[tokio::test]
async fn the_watchdog_signals_after_a_run_of_rejections() {
    // Drives the whole loop — probe, classify, count, signal — against a server
    // that really rejects us, rather than testing the classifier alone.
    let opts = with_wrong_password(&test_db_url());
    if PgConnection::connect_with(&opts).await.is_ok() {
        eprintln!(
            "SKIPPED the_watchdog_signals_after_a_run_of_rejections: server does not \
             enforce passwords (`trust` pg_hba), so no rejection can be produced."
        );
        return;
    }

    let fatal = Arc::new(Notify::new());
    tokio::spawn(hub::db_watchdog::run_watchdog(
        wrong_password_url(&test_db_url()),
        fatal.clone(),
        Duration::from_millis(200),
        2,
    ));

    tokio::time::timeout(Duration::from_secs(15), fatal.notified())
        .await
        .expect("watchdog must signal after 2 consecutive rejections");
}

#[tokio::test]
async fn the_watchdog_never_signals_when_the_server_is_merely_unreachable() {
    // The issue #17 guarantee, asserted behaviourally: no matter how many times
    // an unreachable server fails, strikes reset and the hub is never told to die.
    let fatal = Arc::new(Notify::new());
    tokio::spawn(hub::db_watchdog::run_watchdog(
        "postgres://someone:something@127.0.0.1:1/nothing".to_string(),
        fatal.clone(),
        Duration::from_millis(100),
        2,
    ));

    let signalled = tokio::time::timeout(Duration::from_secs(3), fatal.notified()).await;
    assert!(
        signalled.is_err(),
        "an unreachable server must never trigger the exit signal — that would turn a \
         pg1 outage or DNS flake into a crash loop"
    );
}

#[tokio::test]
async fn a_valid_credential_does_not_classify_as_an_auth_failure() {
    // Sanity anchor: the same code path against the real credential succeeds, so
    // the test above is detecting the password and not some unrelated breakage.
    let mut conn = PgConnection::connect(&test_db_url())
        .await
        .expect("the real test credential must connect");
    let one: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&mut conn)
        .await
        .expect("SELECT 1 must succeed");
    assert_eq!(one, 1);
    let _ = conn.close().await;
}
