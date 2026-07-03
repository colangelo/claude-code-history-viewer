//! Session loading commands.
//!
//! The pure loading/parsing logic lives in `history_core::providers::claude`
//! (so a headless daemon can reuse it). These thin Tauri command wrappers keep
//! the desktop command surface unchanged.

use crate::models::{ClaudeMessage, ClaudeSession, MessagePage};

// Re-export the subagent metadata type so existing `crate::commands::session`
// consumers keep resolving it unchanged.
pub use history_core::providers::claude::SubagentSession;

#[tauri::command]
pub async fn load_project_sessions(
    project_path: String,
    exclude_sidechain: Option<bool>,
) -> Result<Vec<ClaudeSession>, String> {
    history_core::providers::claude::load_sessions(&project_path, exclude_sidechain)
}

#[tauri::command]
pub async fn load_session_messages(session_path: String) -> Result<Vec<ClaudeMessage>, String> {
    history_core::providers::claude::load_messages(&session_path)
}

/// Returns subagent sessions for a given parent session file.
#[tauri::command]
pub async fn get_session_subagents(session_path: String) -> Result<Vec<SubagentSession>, String> {
    history_core::providers::claude::subagents(&session_path)
}

#[tauri::command]
pub async fn load_session_messages_paginated(
    session_path: String,
    offset: usize,
    limit: usize,
    exclude_sidechain: Option<bool>,
) -> Result<MessagePage, String> {
    history_core::providers::claude::load_messages_paginated(
        &session_path,
        offset,
        limit,
        exclude_sidechain,
    )
}

#[tauri::command]
pub async fn get_session_message_count(
    session_path: String,
    exclude_sidechain: Option<bool>,
) -> Result<usize, String> {
    history_core::providers::claude::message_count(&session_path, exclude_sidechain)
}
