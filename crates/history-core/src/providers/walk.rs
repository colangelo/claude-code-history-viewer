//! Bounded directory discovery shared by per-project history providers.

use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub(crate) const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_ENTRIES: usize = 20_000;

/// Checked on the readdir name BEFORE any stat: media/system/cloud dirs can be
/// dataless (iCloud, Music library), where even a stat blocks indefinitely,
/// and they never contain code checkouts.
pub(crate) fn is_skipped_dir_name(name: &str, depth: usize) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules"
                | "target"
                | "dist"
                | "build"
                | "Library"
                | "Music"
                | "Movies"
                | "Pictures"
                | "Photos"
                | "Applications"
                | "Public"
        )
        || (depth == 0
            && matches!(
                name,
                "Desktop" // macOS TCC-protected; may contain iCloud dataless items.
                    | "Documents" // macOS TCC-protected; may contain iCloud dataless items.
                    | "Downloads" // macOS TCC-protected; may contain cloud-backed items.
                    | "Creative Cloud Files" // Adobe cloud storage can contain dataless items.
                    | "Dropbox" // Dropbox on-demand storage can block filesystem calls.
                    | "OneDrive" // OneDrive on-demand storage can block filesystem calls.
                    | "Google Drive" // Google Drive streaming can block filesystem calls.
            ))
}

pub(crate) struct WalkBudget {
    max_depth: usize,
    remaining_entries: usize,
    deadline: Instant,
}

impl WalkBudget {
    pub(crate) fn new(max_depth: usize) -> Self {
        Self {
            max_depth,
            remaining_entries: MAX_ENTRIES,
            deadline: Instant::now() + DISCOVERY_TIMEOUT,
        }
    }

    fn exhausted(&self) -> bool {
        self.remaining_entries == 0 || Instant::now() >= self.deadline
    }

    /// `depth` is relative to the search root; home-only skips use depth 0.
    /// The visitor returns false when it has collected enough matches.
    pub(crate) fn walk(
        &mut self,
        root: &Path,
        depth: usize,
        visit: &mut impl FnMut(&Path) -> bool,
    ) -> bool {
        if self.exhausted() || depth > self.max_depth {
            return false;
        }
        // Validate only the explicit root; readdir file types guard descendants.
        if !fs::symlink_metadata(root)
            .map(|m| m.file_type().is_dir())
            .unwrap_or(false)
        {
            return true;
        }
        let skip_depth = usize::from(dirs::home_dir().as_deref() != Some(root));
        self.walk_recursive(root, depth, skip_depth, visit)
    }

    fn walk_recursive(
        &mut self,
        dir: &Path,
        depth: usize,
        skip_depth: usize,
        visit: &mut impl FnMut(&Path) -> bool,
    ) -> bool {
        if Instant::now() >= self.deadline || !visit(dir) {
            return false;
        }
        if depth >= self.max_depth || self.exhausted() {
            return true;
        }
        let Ok(mut entries) = fs::read_dir(dir) else {
            return true;
        };
        loop {
            if self.exhausted() {
                return false;
            }
            let Some(entry) = entries.next() else {
                return true;
            };
            self.remaining_entries -= 1;
            if Instant::now() >= self.deadline {
                return false;
            }
            let Ok(entry) = entry else {
                continue;
            };
            let name = entry.file_name();
            if is_skipped_dir_name(&name.to_string_lossy(), skip_depth) {
                continue;
            }
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            if !self.walk_recursive(&entry.path(), depth + 1, skip_depth + 1, visit) {
                return false;
            }
        }
    }
}

/// Bound the caller's wait by running discovery on a detached thread.
/// On timeout the thread is left running: it may be stuck in an uncancellable
/// kernel filesystem call, so joining it would defeat the timeout.
pub(crate) fn run_with_timeout<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(f());
    });
    receiver.recv_timeout(timeout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn add_db(project: &Path) -> PathBuf {
        let db = project.join(".crush/crush.db");
        fs::create_dir_all(db.parent().unwrap()).unwrap();
        fs::write(&db, b"x").unwrap();
        db
    }

    fn collect_dbs(root: &Path, budget: &mut WalkBudget) -> Vec<PathBuf> {
        let mut found = Vec::new();
        // A temporary root stands in for HOME without changing process-wide env.
        budget.walk_recursive(root, 0, 0, &mut |dir| {
            let db = dir.join(".crush/crush.db");
            if db.is_file() {
                found.push(db);
            }
            true
        });
        found.sort();
        found
    }

    #[test]
    fn skipped_directories_are_not_searched() {
        let tmp = TempDir::new().unwrap();
        for name in [
            ".hidden",
            "node_modules",
            "target",
            "dist",
            "build",
            "Library",
            "Music",
            "Movies",
            "Pictures",
            "Photos",
            "Applications",
            "Public",
            "Desktop",
            "Documents",
            "Downloads",
            "Creative Cloud Files",
            "Dropbox",
            "OneDrive",
            "Google Drive",
        ] {
            add_db(&tmp.path().join(name));
        }
        let normal = add_db(&tmp.path().join("myproject"));
        assert_eq!(collect_dbs(tmp.path(), &mut WalkBudget::new(4)), [normal]);
    }

    #[test]
    fn home_only_skips_allow_nested_directories() {
        let tmp = TempDir::new().unwrap();
        let nested = add_db(&tmp.path().join("projects/Documents"));
        assert_eq!(collect_dbs(tmp.path(), &mut WalkBudget::new(4)), [nested]);
        for name in [
            "Desktop",
            "Documents",
            "Downloads",
            "Creative Cloud Files",
            "Dropbox",
            "OneDrive",
            "Google Drive",
        ] {
            assert!(is_skipped_dir_name(name, 0));
            assert!(!is_skipped_dir_name(name, 1));
        }
    }

    #[test]
    fn non_home_root_allows_documents() {
        let tmp = TempDir::new().unwrap();
        let expected = add_db(&tmp.path().join("Documents"));
        let mut found = Vec::new();
        WalkBudget::new(1).walk(tmp.path(), 0, &mut |dir| {
            let db = dir.join(".crush/crush.db");
            if db.is_file() {
                found.push(db);
            }
            true
        });
        assert_eq!(found, [expected]);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directories_are_not_followed() {
        let tmp = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        add_db(external.path());
        std::os::unix::fs::symlink(external.path(), tmp.path().join("linked")).unwrap();
        assert_eq!(
            collect_dbs(tmp.path(), &mut WalkBudget::new(4)),
            Vec::<PathBuf>::new()
        );
        let mut visited = false;
        WalkBudget::new(4).walk(&tmp.path().join("linked"), 0, &mut |_| {
            visited = true;
            true
        });
        assert!(!visited);
    }

    #[test]
    fn entry_budget_stops_walk() {
        let tmp = TempDir::new().unwrap();
        for name in ["one", "two", "three"] {
            add_db(&tmp.path().join(name));
        }
        assert_eq!(collect_dbs(tmp.path(), &mut WalkBudget::new(4)).len(), 3);
        let mut budget = WalkBudget::new(4);
        budget.remaining_entries = 1;
        assert_eq!(collect_dbs(tmp.path(), &mut budget).len(), 1);
        assert_eq!(budget.remaining_entries, 0);
    }

    #[test]
    fn deadline_stops_walk() {
        let tmp = TempDir::new().unwrap();
        add_db(tmp.path());
        let mut budget = WalkBudget::new(4);
        budget.deadline = Instant::now();
        assert_eq!(collect_dbs(tmp.path(), &mut budget), Vec::<PathBuf>::new());
    }

    #[test]
    fn depth_limit_stops_descent() {
        let tmp = TempDir::new().unwrap();
        let shallow = add_db(&tmp.path().join("one"));
        add_db(&tmp.path().join("one/two"));
        assert_eq!(collect_dbs(tmp.path(), &mut WalkBudget::new(1)), [shallow]);
    }

    #[test]
    fn timeout_returns_without_waiting_for_worker() {
        let started = Instant::now();
        let result = run_with_timeout(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(2));
            42
        });
        assert_eq!(result, None);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn timeout_returns_fast_result() {
        assert_eq!(run_with_timeout(Duration::from_secs(1), || 42), Some(42));
    }
}
