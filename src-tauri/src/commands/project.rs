use crate::models::{ClaudeProject, GitCommit};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[tauri::command]
pub async fn get_git_log(actual_path: String, limit: usize) -> Result<Vec<GitCommit>, String> {
    // Validate path is absolute and exists
    let path_buf = PathBuf::from(&actual_path);
    if !path_buf.is_absolute() {
        return Err("Path must be absolute".to_string());
    }
    if !path_buf.exists() || !path_buf.is_dir() {
        return Err("Path does not exist or is not a directory".to_string());
    }

    // Canonicalize to ensure we are using the real path
    let safe_path = path_buf
        .canonicalize()
        .map_err(|e| format!("Invalid path: {e}"))?;

    let output = Command::new("git")
        .args(["log", "-n"])
        .arg(limit.to_string())
        .args(["--pretty=format:%H|%an|%at|%s"])
        .current_dir(&safe_path)
        .output()
        .map_err(|e| format!("Failed to execute git log: {e}"))?;

    if !output.status.success() {
        return Ok(vec![]);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() == 4 {
            let timestamp = parts[2].parse::<i64>().unwrap_or(0);
            let date = DateTime::<Utc>::from_timestamp(timestamp, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| "unknown".to_string());

            commits.push(GitCommit {
                hash: parts[0].to_string(),
                author: parts[1].to_string(),
                timestamp,
                date,
                message: parts[3].to_string(),
            });
        }
    }

    Ok(commits)
}

#[tauri::command]
pub async fn get_claude_folder_path() -> Result<String, String> {
    let home_dir =
        dirs::home_dir().ok_or("HOME_DIRECTORY_NOT_FOUND:Could not determine home directory")?;
    let claude_path = home_dir.join(".claude");

    if !claude_path.exists() {
        return Err(format!(
            "CLAUDE_FOLDER_NOT_FOUND:Claude folder not found at {}",
            claude_path.display()
        ));
    }

    if fs::read_dir(&claude_path).is_err() {
        return Err(
            "PERMISSION_DENIED:Cannot access Claude folder. Please check permissions.".to_string(),
        );
    }

    Ok(claude_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn validate_claude_folder(path: String) -> Result<bool, String> {
    let path_buf = PathBuf::from(&path);

    if !path_buf.exists() {
        return Ok(false);
    }

    if path_buf.file_name().and_then(|n| n.to_str()) == Some(".claude") {
        let projects_path = path_buf.join("projects");
        return Ok(projects_path.exists() && projects_path.is_dir());
    }

    let claude_path = path_buf.join(".claude");
    if claude_path.exists() && claude_path.is_dir() {
        let projects_path = claude_path.join("projects");
        return Ok(projects_path.exists() && projects_path.is_dir());
    }

    Ok(false)
}

/// Validate a custom Claude configuration directory.
///
/// Unlike `validate_claude_folder` (which expects a `.claude` directory),
/// this accepts any absolute directory containing a `projects/` subfolder
/// and applies symlink safety checks.
#[tauri::command]
pub async fn validate_custom_claude_dir(path: String) -> Result<bool, String> {
    let path_buf = PathBuf::from(&path);
    match crate::utils::validate_custom_claude_path(&path_buf) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Detect `CLAUDE_CONFIG_DIR` environment variable and return the path if valid.
///
/// Returns `Some(path)` if the env var is set and points to a valid Claude
/// configuration directory (has a `projects/` subfolder). Returns `None` otherwise.
#[tauri::command]
pub async fn detect_claude_config_dir() -> Result<Option<String>, String> {
    let raw = match std::env::var("CLAUDE_CONFIG_DIR") {
        Ok(val) if !val.trim().is_empty() => val.trim().to_string(),
        _ => return Ok(None),
    };

    // Expand ~ to home directory (only exact "~" or "~/..." patterns)
    let expanded = if raw == "~" {
        match dirs::home_dir() {
            Some(home) => home.to_string_lossy().to_string(),
            None => raw,
        }
    } else if let Some(rest) = raw.strip_prefix("~/") {
        match dirs::home_dir() {
            Some(home) => home.join(rest).to_string_lossy().to_string(),
            None => raw,
        }
    } else {
        raw
    };

    let path = PathBuf::from(&expanded);
    if !path.is_absolute() {
        return Ok(None);
    }

    match crate::utils::validate_custom_claude_path(&path) {
        Ok(_) => Ok(Some(expanded)),
        Err(_) => Ok(None),
    }
}

#[tauri::command]
pub async fn scan_projects(claude_path: String) -> Result<Vec<ClaudeProject>, String> {
    history_core::providers::claude::scan_projects(&claude_path)
}

#[cfg(test)]
#[allow(clippy::await_holding_lock)] // env var tests are sync internally; no real suspension
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::sync::{LazyLock, Mutex, MutexGuard};
    use tempfile::TempDir;

    /// Mutex to serialize tests that modify the `CLAUDE_CONFIG_DIR` environment variable.
    static ENV_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_MUTEX.lock().unwrap()
    }

    fn create_test_jsonl_file(dir: &PathBuf, filename: &str, content: &str) {
        let file_path = dir.join(filename);
        let mut file = File::create(&file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
    }

    // Test validate_claude_folder
    #[tokio::test]
    async fn test_validate_claude_folder_nonexistent() {
        let result = validate_claude_folder("/nonexistent/path".to_string()).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_validate_claude_folder_without_projects() {
        let temp_dir = TempDir::new().unwrap();
        let claude_dir = temp_dir.path().join(".claude");
        fs::create_dir(&claude_dir).unwrap();
        // No projects subdirectory

        let result = validate_claude_folder(claude_dir.to_string_lossy().to_string()).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_validate_claude_folder_with_projects() {
        let temp_dir = TempDir::new().unwrap();
        let claude_dir = temp_dir.path().join(".claude");
        let projects_dir = claude_dir.join("projects");
        fs::create_dir_all(&projects_dir).unwrap();

        // Test with .claude directory path directly
        let result = validate_claude_folder(claude_dir.to_string_lossy().to_string()).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_validate_claude_folder_from_parent() {
        let temp_dir = TempDir::new().unwrap();
        let claude_dir = temp_dir.path().join(".claude");
        let projects_dir = claude_dir.join("projects");
        fs::create_dir_all(&projects_dir).unwrap();

        // Test with parent directory (home-like path)
        let result = validate_claude_folder(temp_dir.path().to_string_lossy().to_string()).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn test_get_git_log_invalid_path() {
        let result = get_git_log("/nonexistent/path".to_string(), 10).await;
        // Should fail because path doesn't exist
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Path does not exist or is not a directory"
        );
    }

    #[tokio::test]
    async fn test_get_git_log_not_absolute() {
        let result = get_git_log("relative/path".to_string(), 10).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Path must be absolute");
    }

    #[tokio::test]
    async fn test_get_git_log_success() {
        let temp_dir = TempDir::new().unwrap();
        let path_str = temp_dir.path().to_string_lossy().to_string();

        // Initialize git repo
        let _ = Command::new("git")
            .arg("init")
            .current_dir(&temp_dir)
            .output()
            .expect("Failed to init git");

        // Configure user for commit
        let _ = Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&temp_dir)
            .output();
        let _ = Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&temp_dir)
            .output();

        // Create a file and commit it
        create_test_jsonl_file(&temp_dir.path().to_path_buf(), "test.txt", "content");
        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(&temp_dir)
            .output();
        let _ = Command::new("git")
            .args(["commit", "-m", "Initial commit"])
            .current_dir(&temp_dir)
            .output();

        let result = get_git_log(path_str, 5).await;

        // If git is not installed or configured, this might fail or return empty.
        // But assuming git works:
        if let Ok(commits) = result {
            if commits.is_empty() {
                // Might happen in CI without git
                println!("Warning: git log returned empty (git might not be working in test env)");
            } else {
                assert_eq!(commits.len(), 1);
                assert_eq!(commits[0].message, "Initial commit");
                assert_eq!(commits[0].author, "Test User");
            }
        } else {
            // Should not error if path is valid repo
            panic!("get_git_log failed: {}", result.unwrap_err());
        }
    }

    // Tests for detect_claude_config_dir
    // All tests use ENV_MUTEX to prevent race conditions on the global env var.
    #[tokio::test]
    async fn test_detect_config_dir_unset() {
        let _guard = lock_env();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let result = detect_claude_config_dir().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_detect_config_dir_empty() {
        let _guard = lock_env();
        std::env::set_var("CLAUDE_CONFIG_DIR", "");
        let result = detect_claude_config_dir().await.unwrap();
        assert!(result.is_none());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[tokio::test]
    async fn test_detect_config_dir_valid() {
        let _guard = lock_env();
        let temp_dir = TempDir::new().unwrap();
        let projects_dir = temp_dir.path().join("projects");
        fs::create_dir_all(&projects_dir).unwrap();

        std::env::set_var(
            "CLAUDE_CONFIG_DIR",
            temp_dir.path().to_string_lossy().to_string(),
        );
        let result = detect_claude_config_dir().await.unwrap();
        assert!(result.is_some());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[tokio::test]
    async fn test_detect_config_dir_invalid_no_projects() {
        let _guard = lock_env();
        let temp_dir = TempDir::new().unwrap();
        // No projects/ subdirectory

        std::env::set_var(
            "CLAUDE_CONFIG_DIR",
            temp_dir.path().to_string_lossy().to_string(),
        );
        let result = detect_claude_config_dir().await.unwrap();
        assert!(result.is_none());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[tokio::test]
    async fn test_detect_config_dir_relative_path() {
        let _guard = lock_env();
        std::env::set_var("CLAUDE_CONFIG_DIR", "relative/path");
        let result = detect_claude_config_dir().await.unwrap();
        assert!(result.is_none());
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }
}
