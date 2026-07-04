//! Debounced file-watching: turns filesystem activity under the provider
//! roots into "run a sync pass soon" signals, so new messages reach the hub
//! in seconds instead of waiting for the hourly safety-net rescan.
//!
//! The watcher is a latency optimization only — correctness always comes from
//! the periodic rescan plus the hub's idempotent ingest (see the
//! `history-sync-daemon` spec). A missed event is therefore never a bug, and
//! failures here must degrade to rescan-only behavior, never crash the daemon.
//!
//! STUB: public surface committed ahead of implementation so acceptance evals
//! compile against it; `spawn` currently registers nothing and never signals.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

/// Keeps the underlying filesystem watcher alive; dropping it stops watching.
pub struct WatcherGuard {
    _private: (),
}

/// Start watching `roots` (recursively) with the given debounce window.
///
/// Returns a guard plus a channel that yields one unit signal per debounced
/// burst of filesystem activity. Roots that cannot be watched (missing,
/// permission denied) are logged and skipped — as long as the watcher itself
/// can start, this returns `Ok` and the remaining roots are watched.
pub fn spawn(
    roots: &[PathBuf],
    debounce: Duration,
) -> anyhow::Result<(WatcherGuard, mpsc::Receiver<()>)> {
    let _ = (roots, debounce);
    let (_tx, rx) = mpsc::channel(1);
    Ok((WatcherGuard { _private: () }, rx))
}

/// Coalesces watcher signals into bounded-rate sync passes: at most one pass
/// per `min_gap`, but a trigger arriving inside the gap is remembered so a
/// burst always ends with a pass.
pub struct PassThrottle {
    min_gap: Duration,
}

impl PassThrottle {
    #[must_use]
    pub fn new(min_gap: Duration) -> Self {
        Self { min_gap }
    }

    /// Record that a watcher signal arrived at `now`.
    pub fn note_trigger(&mut self, now: Instant) {
        let _ = now;
    }

    /// Record that a sync pass completed at `now`.
    pub fn note_pass(&mut self, now: Instant) {
        let _ = now;
    }

    /// Whether a pass should run at `now`: true iff a trigger is pending and
    /// at least `min_gap` has elapsed since the last pass. Consumes the
    /// pending trigger when it returns true.
    pub fn pass_due(&mut self, now: Instant) -> bool {
        let _ = (now, self.min_gap);
        false
    }
}
