//! Tauri command wrappers for Antigravity token-state.
//!
//! The pure state-building logic was extracted to `history_core::antigravity`.
//! This module re-exports it (so existing `crate::commands::antigravity::*`
//! paths keep resolving — e.g. from `commands::stats`) and adds the thin
//! `#[tauri::command]` IPC wrappers.

pub use history_core::antigravity::*;

use history_core::models::{AntigravityProjectSummary, AntigravityState, PersistedSessionState};

#[tauri::command]
pub async fn load_antigravity_state() -> Result<AntigravityState, String> {
    let root = resolve_antigravity_root().ok_or("Cannot determine antigravity root directory")?;
    load_antigravity_state_impl(&root)
}

#[tauri::command]
pub async fn get_antigravity_session(
    session_id: String,
) -> Result<Option<PersistedSessionState>, String> {
    if !is_valid_antigravity_session_id(&session_id) {
        return Err("Invalid antigravity session_id".to_string());
    }
    let root = resolve_antigravity_root().ok_or("Cannot determine antigravity root directory")?;
    let state = load_antigravity_state_impl(&root)?;
    Ok(state.sessions.get(&session_id).cloned())
}

#[tauri::command]
pub async fn get_antigravity_project_summary(
    root_path: Option<String>,
) -> Result<AntigravityProjectSummary, String> {
    // Resolution order: marker-anchored root from the supplied path, then the
    // platform-discovered default. Accepting `PathBuf::from(root_path)` directly
    // would weaken the marker contract, so it is intentionally omitted.
    let root = root_path
        .as_deref()
        .and_then(antigravity_root_from_path)
        .or_else(resolve_antigravity_root)
        .ok_or("Cannot determine antigravity root directory")?;
    let state = load_antigravity_state_impl(&root)?;
    Ok(compute_project_summary(&state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Command-level test (moved with its `#[tauri::command]` wrapper from the
    // history-core extraction). Pure-helper tests live in `history_core::antigravity`.
    #[tokio::test]
    async fn test_get_antigravity_project_summary_uses_explicit_root_path() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let rpc_dir = root
            .join(".token-monitor")
            .join("rpc-cache")
            .join("v1")
            .join("sess-explicit-root");
        std::fs::create_dir_all(&rpc_dir).unwrap();
        std::fs::create_dir_all(root.join("brain").join("sess-explicit-root")).unwrap();

        let usage = r#"{"recordType":"usage","sessionId":"sess-explicit-root","sequence":0,"model":"gemini-3-pro-high","inputTokens":120,"outputTokens":80,"cacheReadTokens":40,"cacheWriteTokens":20,"reasoningTokens":10,"totalTokens":270,"raw":{"chatModel":{"chatStartMetadata":{"createdAt":"2026-04-12T00:00:00Z"}}}}"#;
        std::fs::write(rpc_dir.join("usage.jsonl"), format!("{usage}\n")).unwrap();

        let summary = get_antigravity_project_summary(Some(root.to_string_lossy().to_string()))
            .await
            .unwrap();

        assert_eq!(summary.session_count, 1);
        assert_eq!(summary.total_tokens, 270);
        assert_eq!(summary.sessions[0].session_id, "sess-explicit-root");
    }
}
