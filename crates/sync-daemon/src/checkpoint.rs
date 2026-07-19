//! Crash-safe sync checkpoint.
//!
//! Records, per source session file, the file size + mtime + message count + a
//! timestamp at the moment the hub acknowledged it. A file is considered
//! unchanged (and skipped) when its current size and mtime match the
//! checkpoint, so re-runs never re-send already-acknowledged data. Persisted
//! atomically so a crash mid-write cannot corrupt it.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::fs_atomic::write_atomic;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    pub size: u64,
    pub mtime_ms: u64,
    pub message_count: usize,
    pub last_synced_ms: u64,
}

/// Consecutive whole-session delivery failures attempted at full cost before
/// the session is put on a backoff schedule. Small, so a transient hub blip
/// still retries on the very next pass.
pub const FAILURE_GRACE: u32 = 3;

/// First backoff step once the grace window is spent; doubles per further
/// failure up to `BACKOFF_MAX_SECS`.
const BACKOFF_BASE_SECS: u64 = 15 * 60;
const BACKOFF_MAX_SECS: u64 = 24 * 60 * 60;

/// A session that fails delivery *every* pass, forever, is the expensive case:
/// each attempt burns the full retry ladder (minutes of wall clock), and 40-odd
/// of them stretched an m4m sync pass from seconds to ~70 minutes (2026-07-19
/// retry-backlog report). Recording the failure streak lets those sessions back
/// off instead of being retried at full cost on every single pass — they are
/// still retried, just on a widening schedule, and any edit to the file resets
/// the streak (new bytes deserve a fresh try).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FailState {
    pub consecutive: u32,
    pub last_attempt_ms: u64,
    /// The file's size/mtime at the last failure — a change resets the streak.
    pub size: u64,
    pub mtime_ms: u64,
}

/// Backoff for a given streak length, or `None` while still inside the grace
/// window (retry on the next pass).
fn backoff_secs(consecutive: u32) -> Option<u64> {
    if consecutive < FAILURE_GRACE {
        return None;
    }
    let shift = (consecutive - FAILURE_GRACE).min(16);
    Some(
        BACKOFF_BASE_SECS
            .saturating_mul(1u64 << shift)
            .min(BACKOFF_MAX_SECS),
    )
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Keyed by absolute session file path.
    pub files: HashMap<String, FileState>,
    /// Delivery-failure streaks, keyed by absolute session file path. Absent for
    /// every healthy session, so the common case costs nothing.
    #[serde(default)]
    pub failures: HashMap<String, FailState>,
    #[serde(skip)]
    path: PathBuf,
}

impl Checkpoint {
    /// Load from `<state_dir>/checkpoint.json`, or start empty. An unreadable or
    /// corrupt checkpoint is treated as empty (a full rescan is safe because the
    /// hub ingest is idempotent).
    pub fn load(state_dir: &Path) -> Self {
        let path = state_dir.join("checkpoint.json");
        let mut cp: Checkpoint = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        cp.path = path;
        cp
    }

    /// Does this file match its checkpoint (same size + mtime)? Unknown → false.
    pub fn is_unchanged(&self, file: &str, size: u64, mtime_ms: u64) -> bool {
        self.files
            .get(file)
            .is_some_and(|s| s.size == size && s.mtime_ms == mtime_ms)
    }

    pub fn record(&mut self, file: String, state: FileState) {
        // A successful delivery clears any failure streak for this file.
        self.failures.remove(&file);
        self.files.insert(file, state);
    }

    /// Seconds still to wait before this session is worth attempting again, or
    /// `None` when it should be attempted now. A file whose size or mtime moved
    /// since the last failure is always attempted (streak reset happens on the
    /// next `note_failure`).
    pub fn defer_remaining_secs(
        &self,
        file: &str,
        size: u64,
        mtime_ms: u64,
        now_ms: u64,
    ) -> Option<u64> {
        let fail = self.failures.get(file)?;
        if fail.size != size || fail.mtime_ms != mtime_ms {
            return None;
        }
        let wait = backoff_secs(fail.consecutive)?;
        let elapsed = now_ms.saturating_sub(fail.last_attempt_ms) / 1000;
        wait.checked_sub(elapsed).filter(|&r| r > 0)
    }

    /// Record a failed delivery. Returns the new streak length. A file that
    /// changed since the last failure starts a fresh streak.
    pub fn note_failure(&mut self, file: &str, size: u64, mtime_ms: u64, now_ms: u64) -> u32 {
        let entry = self.failures.entry(file.to_string()).or_default();
        let changed = entry.size != size || entry.mtime_ms != mtime_ms;
        entry.consecutive = if changed { 1 } else { entry.consecutive + 1 };
        entry.size = size;
        entry.mtime_ms = mtime_ms;
        entry.last_attempt_ms = now_ms;
        entry.consecutive
    }

    pub fn save(&self) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).unwrap_or_default();
        write_atomic(&self.path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: u64 = 1000;

    fn cp() -> Checkpoint {
        Checkpoint::default()
    }

    #[test]
    fn no_deferral_without_a_failure_streak() {
        assert_eq!(cp().defer_remaining_secs("f", 1, 2, 0), None);
    }

    #[test]
    fn grace_window_retries_on_the_next_pass() {
        let mut c = cp();
        for expected in 1..FAILURE_GRACE {
            assert_eq!(c.note_failure("f", 1, 2, 0), expected);
            assert_eq!(
                c.defer_remaining_secs("f", 1, 2, 0),
                None,
                "streak {expected} must still retry immediately"
            );
        }
    }

    #[test]
    fn past_the_grace_window_the_session_backs_off_and_widens() {
        let mut c = cp();
        for _ in 0..FAILURE_GRACE {
            c.note_failure("f", 1, 2, 0);
        }
        assert_eq!(
            c.defer_remaining_secs("f", 1, 2, 0),
            Some(BACKOFF_BASE_SECS)
        );
        // Waiting it out clears the deferral.
        assert_eq!(
            c.defer_remaining_secs("f", 1, 2, BACKOFF_BASE_SECS * SEC),
            None
        );
        // The next failure doubles the wait.
        c.note_failure("f", 1, 2, BACKOFF_BASE_SECS * SEC);
        assert_eq!(
            c.defer_remaining_secs("f", 1, 2, BACKOFF_BASE_SECS * SEC),
            Some(BACKOFF_BASE_SECS * 2)
        );
    }

    #[test]
    fn backoff_is_capped() {
        assert_eq!(backoff_secs(200), Some(BACKOFF_MAX_SECS));
    }

    #[test]
    fn an_edited_file_is_always_retried_and_resets_the_streak() {
        let mut c = cp();
        for _ in 0..10 {
            c.note_failure("f", 1, 2, 0);
        }
        assert!(c.defer_remaining_secs("f", 1, 2, 0).is_some());
        // Same path, new bytes → attempt now, and the streak restarts at 1.
        assert_eq!(c.defer_remaining_secs("f", 9, 9, 0), None);
        assert_eq!(c.note_failure("f", 9, 9, 0), 1);
    }

    #[test]
    fn a_successful_delivery_clears_the_streak() {
        let mut c = cp();
        for _ in 0..10 {
            c.note_failure("f", 1, 2, 0);
        }
        c.record(
            "f".into(),
            FileState {
                size: 1,
                mtime_ms: 2,
                message_count: 3,
                last_synced_ms: 0,
            },
        );
        assert!(c.failures.is_empty());
        assert_eq!(c.defer_remaining_secs("f", 1, 2, 0), None);
    }

    #[test]
    fn an_old_checkpoint_without_failures_still_loads() {
        let json =
            r#"{"files":{"f":{"size":1,"mtime_ms":2,"message_count":3,"last_synced_ms":4}}}"#;
        let c: Checkpoint = serde_json::from_str(json).expect("legacy checkpoint must parse");
        assert_eq!(c.files.len(), 1);
        assert!(c.failures.is_empty());
    }
}
