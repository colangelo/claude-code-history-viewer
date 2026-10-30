//! Extra Claude config folders (`claude_extra_roots`): sessions written by a
//! Claude Code run with its own `CLAUDE_CONFIG_DIR` live outside `~/.claude`,
//! and only folders named here are archived. Resolution happens once at
//! start-up and never fails: a root that cannot be used is skipped with a
//! warning, and a root that is the main folder (or one already listed) is
//! skipped so no transcript is read twice.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Expand a leading `~` to `home`, canonicalize, and drop every root that is
/// missing, relative, the main Claude folder `main`, or a repeat.
#[must_use]
pub fn resolve(raw: &[String], home: &Path, main: Option<&Path>) -> Vec<PathBuf> {
    let main = main.and_then(|m| m.canonicalize().ok());
    let mut seen: HashSet<PathBuf> = main.into_iter().collect();
    let mut roots = Vec::new();
    for entry in raw {
        let expanded = match entry.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                home.join(rest.trim_start_matches('/'))
            }
            _ => PathBuf::from(entry),
        };
        if !expanded.is_absolute() {
            tracing::warn!(root = %entry, "claude_extra_roots: not an absolute path, skipped");
            continue;
        }
        let Ok(canonical) = expanded.canonicalize() else {
            tracing::warn!(root = %entry, "claude_extra_roots: folder does not exist, skipped");
            continue;
        };
        if !seen.insert(canonical.clone()) {
            tracing::warn!(root = %entry, "claude_extra_roots: same folder as the main Claude folder or an earlier root, skipped");
            continue;
        }
        roots.push(canonical);
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn tilde_expands_to_home_and_the_folder_is_kept() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".config/claude-hlp-b")).unwrap();
        let got = resolve(&s(&["~/.config/claude-hlp-b"]), home.path(), None);
        assert_eq!(
            got,
            vec![home
                .path()
                .join(".config/claude-hlp-b")
                .canonicalize()
                .unwrap()]
        );
    }

    #[test]
    fn missing_and_relative_roots_are_skipped() {
        let home = tempfile::tempdir().unwrap();
        let got = resolve(
            &s(&["~/.config/nope", "relative/claude"]),
            home.path(),
            None,
        );
        assert!(got.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_the_main_folder_and_a_repeat_are_skipped() {
        let home = tempfile::tempdir().unwrap();
        let main = home.path().join(".config/claude");
        let extra = home.path().join(".config/claude-hlp-b");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        std::os::unix::fs::symlink(&main, home.path().join(".claude")).unwrap();
        let got = resolve(
            &s(&[
                "~/.claude",
                "~/.config/claude-hlp-b",
                "~/.config/claude-hlp-b/",
            ]),
            home.path(),
            Some(&main),
        );
        assert_eq!(got, vec![extra.canonicalize().unwrap()]);
    }
}
