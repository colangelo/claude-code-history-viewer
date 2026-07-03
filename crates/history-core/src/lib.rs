//! Tauri-free extraction and normalization of AI coding-agent conversation history.
//!
//! This crate owns provider detection and the parse/normalize pipeline shared by
//! the desktop app (`src-tauri`) and the sync daemon. The normalized models
//! (`ClaudeMessage`, `ClaudeSession`, `ClaudeProject`) and the per-provider
//! parsers were extracted here so headless binaries can reuse them.
//!
//! Invariant: this crate MUST NOT depend on `tauri` or any GUI/webview crate.

pub mod antigravity;
pub mod cli_args;
pub mod export;
pub mod fs_utils;
pub mod models;
pub mod providers;
pub mod search_text;
pub mod utils;
