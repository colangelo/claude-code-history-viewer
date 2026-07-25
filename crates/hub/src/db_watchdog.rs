//! Database credential watchdog.
//!
//! The hub resolves its Postgres password once, at process start, but that
//! password is owned and rotated by `OpenBao`. When it rotates, the running hub
//! keeps serving on its already-open pool and then fails every new connection —
//! and nothing exits, so it sits returning 500s until a human intervenes.
//!
//! This watchdog closes that gap by making the process *exit*, which is all the
//! recovery that is needed: `dev.cchv.hub` runs `cchv-launch hub` under launchd
//! `KeepAlive`, so an exit is followed by a relaunch that re-reads the rotating
//! credential from bao. Recovery therefore lives in the supervisor, and this
//! module deliberately does NOT attempt in-process credential renewal.
//!
//! Two properties matter more than anything else here:
//!
//! 1. **Only an authentication rejection counts.** Postgres being unreachable,
//!    DNS failing, a pool timeout, or any other SQLSTATE resets the counter.
//!    Restarting the hub cannot fix a down database, and a transient `MagicDNS`
//!    flake against pg1 must never exit the process (that is issue #17, whose
//!    `min_connections(2)` mitigation stays intact).
//! 2. **Each probe uses a fresh connection.** Borrowing from the pool would
//!    mask the very thing we are looking for: pooled connections keep
//!    authenticating until they age out, so a pool-based probe stays green while
//!    real requests that need a new connection fail. Probing fresh means the hub
//!    restarts *before* it degrades rather than after.
//!
//! The credential itself is never logged.

use std::sync::Arc;
use std::time::Duration;

use sqlx::{Connection, PgConnection};
use tokio::sync::Notify;

/// Postgres SQLSTATE `invalid_password` — the one code that counts as a
/// credential rejection.
const INVALID_PASSWORD: &str = "28P01";

/// How often to probe.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(30);

/// Consecutive authentication failures required before exiting. Three probes at
/// the default interval means ~90 s from rotation to exit, well inside the
/// plist's `ThrottleInterval=300` so a relaunch never hits respawn throttling.
pub const DEFAULT_STRIKE_LIMIT: u32 = 3;

/// Per-probe ceiling, so a connection attempt that hangs cannot stall the loop.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// What one probe established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Authenticated and queried successfully.
    Healthy,
    /// Postgres rejected our credential.
    AuthRejected,
    /// Anything else: unreachable, timed out, or any non-auth database error.
    /// Deliberately not distinguished further — none of it counts toward exit.
    Unavailable,
}

/// Does this SQLSTATE mean "your credential was rejected"?
///
/// Split out from [`is_auth_failure`] on purpose: this predicate is the entire
/// safety boundary of the module, and `sqlx`'s `PgDatabaseError` cannot be
/// constructed in a unit test (its fields are private and it is only produced
/// from a real server handshake). Taking a bare code keeps the decision fully
/// testable without a database.
pub fn is_auth_code(code: Option<&str>) -> bool {
    code == Some(INVALID_PASSWORD)
}

/// Adapter from a real `sqlx` error to [`is_auth_code`]. Kept to a single
/// `matches!` so that no logic can hide in the layer tests cannot reach.
pub fn is_auth_failure(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if is_auth_code(db.code().as_deref()))
}

/// The consecutive-failure count after observing one probe.
///
/// An authentication rejection extends the run; **every** other outcome ends it.
pub fn next_strikes(current: u32, auth_rejected: bool) -> u32 {
    if auth_rejected {
        current.saturating_add(1)
    } else {
        0
    }
}

/// Open a connection of our own, prove we can authenticate and query, close it.
///
/// Not taken from the pool — see the module docs. Never logs the URL.
async fn probe(database_url: &str) -> Result<(), sqlx::Error> {
    let mut conn = PgConnection::connect(database_url).await?;
    let queried = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&mut conn)
        .await
        .map(|_| ());
    // Best-effort: a failed close tells us nothing about the credential.
    let _ = conn.close().await;
    queried
}

/// One timeout-bounded probe, classified.
async fn probe_once(database_url: &str) -> ProbeOutcome {
    match tokio::time::timeout(PROBE_TIMEOUT, probe(database_url)).await {
        Ok(Ok(())) => ProbeOutcome::Healthy,
        Ok(Err(e)) if is_auth_failure(&e) => ProbeOutcome::AuthRejected,
        // Any non-auth error, or an elapsed timeout: neither is evidence about
        // the credential, so both must leave the strike run broken.
        Ok(Err(_)) | Err(_) => ProbeOutcome::Unavailable,
    }
}

/// Probe until the credential has been rejected `limit` times in a row, then
/// notify `fatal` and stop. The caller owns the exit — see `hub::run`.
///
/// A `limit` of 0 is treated as 1; exiting on zero consecutive failures would
/// mean exiting unconditionally.
pub async fn run_watchdog(
    database_url: String,
    fatal: Arc<Notify>,
    interval: Duration,
    limit: u32,
) {
    let limit = limit.max(1);
    let mut strikes: u32 = 0;

    loop {
        let outcome = probe_once(&database_url).await;
        strikes = next_strikes(strikes, outcome == ProbeOutcome::AuthRejected);

        match outcome {
            ProbeOutcome::AuthRejected => {
                tracing::warn!(strikes, limit, "postgres rejected the hub's credential");
            }
            // Debug, not warn: a pg1 outage or DNS flake can persist for hours,
            // and this is explicitly not a condition we act on.
            ProbeOutcome::Unavailable => {
                tracing::debug!("db watchdog probe failed for a non-credential reason");
            }
            ProbeOutcome::Healthy => {}
        }

        if strikes >= limit {
            tracing::error!(
                strikes,
                "postgres rejected the hub's credential on {strikes} consecutive probes — \
                 it appears to have been rotated; exiting so the supervisor re-resolves it"
            );
            fatal.notify_one();
            return;
        }

        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_invalid_password_is_an_auth_code() {
        assert!(is_auth_code(Some("28P01")));
    }

    #[test]
    fn transient_connect_codes_are_not_auth_codes() {
        // sqlx retries these two internally; they mean "busy"/"starting up",
        // never "wrong password".
        assert!(!is_auth_code(Some("53300"))); // too_many_connections
        assert!(!is_auth_code(Some("57P03"))); // cannot_connect_now
    }

    #[test]
    fn unrelated_and_missing_codes_are_not_auth_codes() {
        assert!(!is_auth_code(Some("42P01"))); // undefined_table
        assert!(!is_auth_code(Some("28000"))); // invalid_authorization_spec
        assert!(!is_auth_code(Some("")));
        assert!(!is_auth_code(None));
    }

    #[test]
    fn auth_rejection_extends_the_run() {
        assert_eq!(next_strikes(0, true), 1);
        assert_eq!(next_strikes(1, true), 2);
        assert_eq!(next_strikes(2, true), 3);
    }

    #[test]
    fn any_other_outcome_resets_the_run() {
        assert_eq!(next_strikes(2, false), 0);
        assert_eq!(next_strikes(0, false), 0);
    }

    #[test]
    fn a_reset_run_restarts_from_one() {
        let after_reset = next_strikes(2, false);
        assert_eq!(next_strikes(after_reset, true), 1);
    }

    #[test]
    fn interleaved_failures_do_not_accumulate() {
        // auth, then a blip, then auth again → 1, not 3.
        let s = next_strikes(0, true);
        let s = next_strikes(s, false);
        let s = next_strikes(s, true);
        assert_eq!(s, 1);
    }

    #[test]
    fn the_limit_is_reached_at_exactly_three_consecutive_rejections() {
        let mut s = 0;
        for _ in 0..2 {
            s = next_strikes(s, true);
        }
        assert!(
            s < DEFAULT_STRIKE_LIMIT,
            "must not fire on the second strike"
        );
        s = next_strikes(s, true);
        assert!(s >= DEFAULT_STRIKE_LIMIT, "must fire on the third");
    }
}
