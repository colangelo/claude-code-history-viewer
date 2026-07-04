//! Debounced file-watching: turns filesystem activity under the provider
//! roots into "run a sync pass soon" signals, so new messages reach the hub
//! in seconds instead of waiting for the hourly safety-net rescan.
//!
//! The watcher is a latency optimization only — correctness always comes from
//! the periodic rescan plus the hub's idempotent ingest (see the
//! `history-sync-daemon` spec). A missed event is therefore never a bug, and
//! failures here must degrade to rescan-only behavior, never crash the daemon.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, Debouncer};
use tokio::sync::mpsc;

/// Keeps the underlying filesystem watcher alive; dropping it stops watching.
pub struct WatcherGuard {
    _debouncer: Debouncer<notify::RecommendedWatcher>,
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
    let (tx, rx) = mpsc::channel(1);

    let mut debouncer = new_debouncer(
        debounce,
        move |result: Result<Vec<DebouncedEvent>, notify::Error>| match result {
            Ok(events) => {
                if !events.is_empty() {
                    // Bounded channel: a dropped send just means a signal is
                    // already pending, which already means "pass soon".
                    let _ = tx.try_send(());
                }
            }
            Err(error) => {
                tracing::warn!(%error, "file watcher error");
            }
        },
    )
    .map_err(|e| anyhow::anyhow!("failed to create file watcher: {e}"))?;

    for root in roots {
        if let Err(error) = debouncer.watcher().watch(root, RecursiveMode::Recursive) {
            tracing::warn!(root = %root.display(), %error, "failed to watch root, skipping");
        }
    }

    Ok((
        WatcherGuard {
            _debouncer: debouncer,
        },
        rx,
    ))
}

/// Coalesces watcher signals into bounded-rate sync passes: at most one pass
/// per `min_gap`, but a trigger arriving inside the gap is remembered so a
/// burst always ends with a pass.
pub struct PassThrottle {
    min_gap: Duration,
    last_pass: Option<Instant>,
    pending: bool,
}

impl PassThrottle {
    #[must_use]
    pub fn new(min_gap: Duration) -> Self {
        Self {
            min_gap,
            last_pass: None,
            pending: false,
        }
    }

    /// Record that a watcher signal arrived at `now`.
    pub fn note_trigger(&mut self, now: Instant) {
        let _ = now;
        self.pending = true;
    }

    /// Record that a sync pass completed at `now`.
    pub fn note_pass(&mut self, now: Instant) {
        self.last_pass = Some(now);
    }

    /// Whether a pass should run at `now`: true iff a trigger is pending and
    /// at least `min_gap` has elapsed since the last pass. Consumes the
    /// pending trigger when it returns true.
    pub fn pass_due(&mut self, now: Instant) -> bool {
        if !self.pending {
            return false;
        }
        let due = match self.last_pass {
            None => true,
            Some(last) => now.saturating_duration_since(last) >= self.min_gap,
        };
        if due {
            self.pending = false;
        }
        due
    }
}
