//! Result of a native (in-place) session rename.
//!
//! Shared across providers (e.g. Codex, `ForgeCode`) that support renaming a
//! session in its own on-disk store, and re-exported by the desktop app's
//! `commands::session` module for backward compatibility.

use serde::{Deserialize, Serialize};

/// Result structure for rename operations.
#[derive(Debug, Serialize, Deserialize)]
pub struct NativeRenameResult {
    pub success: bool,
    pub previous_title: String,
    pub new_title: String,
    pub file_path: String,
}
