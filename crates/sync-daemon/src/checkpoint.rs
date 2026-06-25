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

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Keyed by absolute session file path.
    pub files: HashMap<String, FileState>,
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
        self.files.insert(file, state);
    }

    pub fn save(&self) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).unwrap_or_default();
        write_atomic(&self.path, &bytes)
    }
}
