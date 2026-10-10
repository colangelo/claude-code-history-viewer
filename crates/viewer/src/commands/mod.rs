// These were Tauri commands, whose macro hid `clippy::unused_async`. They stay
// `async` because the WebUI server's handler macros `.await` every command
// uniformly, so a sync signature here would only move the special case there.
#![allow(clippy::unused_async)]

pub mod antigravity;
pub mod archive;
pub mod claude_settings;
pub mod feedback;
pub mod fs_utils;
pub mod mcp_presets;
pub mod metadata;
pub mod multi_provider;
pub mod project;
pub mod session;
pub mod settings;
pub mod stats;
pub mod unified_presets;
pub mod watcher;
pub mod wsl;

#[cfg(test)]
mod proptest_examples;
