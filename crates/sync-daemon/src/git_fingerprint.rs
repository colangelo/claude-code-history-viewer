//! Git fingerprint capture for project identity (fork-owned; deliberately NOT
//! in `history-core`, which is the upstream-sync parser surface).
//!
//! Captures the facts behind the archive's project identity — root commit,
//! normalized `origin` URL, worktree status — by shelling out to `git`.
//! Everything here is defensive: capture runs only when a `.git` marker
//! exists, every subprocess has a hard deadline, and any failure degrades to
//! "no fingerprint" without affecting the sync pass (the crush/aider
//! cloud-dir walk wedge taught us that touching arbitrary user dirs must be
//! time-boxed and non-fatal).

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use archive_protocol::identity::normalize_remote_url;

/// Hard deadline per git subprocess.
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Captured git facts for one project directory.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GitFingerprint {
    /// Full 40-hex root commit (lexicographically smallest when several).
    /// `None` for shallow repos (their visible "roots" are graft points that
    /// vary with clone depth — a false identity) and repos with no commits.
    pub root_commit: Option<String>,
    /// Normalized `origin` URL (`host/path`), credentials stripped.
    pub remote_url: Option<String>,
    /// True when the directory is a linked `git worktree`.
    pub is_worktree: bool,
    /// For linked worktrees: the main checkout's path.
    pub main_path: Option<String>,
}

/// Capture the git fingerprint for a project directory.
///
/// Returns `None` when the directory has no `.git` marker (not a repo).
/// A repo where every fact fails to resolve still returns a (mostly empty)
/// fingerprint — the wire fields simply stay `None`.
pub fn capture(project_dir: &str) -> Option<GitFingerprint> {
    let dir = Path::new(project_dir);
    let marker = dir.join(".git");
    if !marker.exists() {
        return None;
    }

    // Worktree status from the marker shape (`.git` file = linked worktree),
    // same rule as history-core's detect_git_worktree_info.
    let (is_worktree, main_path) = if marker.is_file() {
        (true, worktree_main_path(&marker))
    } else {
        (false, None)
    };

    // Shallow repos would report graft boundaries as roots — skip the root.
    let shallow = run_git(dir, &["rev-parse", "--is-shallow-repository"])
        .map(|out| out.trim() == "true")
        .unwrap_or(true); // can't tell → don't trust the root

    let root_commit = if shallow {
        None
    } else {
        run_git(dir, &["rev-list", "--max-parents=0", "HEAD"]).and_then(|out| {
            out.lines()
                .map(str::trim)
                .filter(|l| l.len() == 40 && l.bytes().all(|b| b.is_ascii_hexdigit()))
                .min() // lexicographically smallest → deterministic
                .map(str::to_string)
        })
    };

    let remote_url = run_git(dir, &["config", "--get", "remote.origin.url"])
        .and_then(|out| normalize_remote_url(out.trim()));

    Some(GitFingerprint {
        root_commit,
        remote_url,
        is_worktree,
        main_path,
    })
}

/// Per-pass memo: multiple providers frequently map to the same directory
/// (claude + codex + pi in one repo), and the fingerprint is identical.
#[derive(Default)]
pub struct FingerprintCache {
    memo: HashMap<String, Option<GitFingerprint>>,
}

impl FingerprintCache {
    pub fn get(&mut self, project_dir: &str) -> Option<GitFingerprint> {
        self.memo
            .entry(project_dir.to_string())
            .or_insert_with(|| capture(project_dir))
            .clone()
    }
}

/// `.git` file content is `gitdir: /main/.git/worktrees/<name>`; the main
/// checkout is the parent of that `.git`. (history-core has the same logic,
/// but private — duplicated here to keep the upstream surface untouched.)
fn worktree_main_path(git_file: &Path) -> Option<String> {
    let content = std::fs::read_to_string(git_file).ok()?;
    let gitdir = content.strip_prefix("gitdir: ")?.trim();
    const MARKER: &str = "/.git/worktrees/";
    let pos = gitdir.find(MARKER)?;
    Some(gitdir[..pos].to_string())
}

/// Run `git -C <dir> <args>` with a hard deadline; `None` on any failure.
fn run_git(dir: &Path, args: &[&str]) -> Option<String> {
    run_with_timeout("git", dir, args, GIT_TIMEOUT)
}

/// Spawn, poll with `try_wait`, kill past the deadline. Output is read only
/// after exit — fine for the tiny outputs involved here (a pathological repo
/// whose root-list overflows the pipe buffer just hits the timeout → `None`).
fn run_with_timeout(program: &str, dir: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .current_dir(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut out = String::new();
                child.stdout.take()?.read_to_string(&mut out).ok()?;
                return Some(out);
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Plain `git` runner for test setup (no timeout ceremony).
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    fn init_repo(dir: &Path) {
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "user.email", "t@example.com"]);
        git(dir, &["config", "user.name", "t"]);
        git(dir, &["commit", "--allow-empty", "-q", "-m", "root"]);
    }

    #[test]
    fn captures_normal_repo_with_remote() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        init_repo(dir);
        git(
            dir,
            &["remote", "add", "origin", "git@github.com:acme/foo.git"],
        );

        let fp = capture(dir.to_str().unwrap()).expect("is a repo");
        let root = fp.root_commit.expect("has root");
        assert_eq!(root.len(), 40);
        assert!(root.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(fp.remote_url.as_deref(), Some("github.com/acme/foo"));
        assert!(!fp.is_worktree);
        assert_eq!(fp.main_path, None);
    }

    #[test]
    fn no_remote_repo_has_root_only() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());

        let fp = capture(tmp.path().to_str().unwrap()).expect("is a repo");
        assert!(fp.root_commit.is_some());
        assert_eq!(fp.remote_url, None);
    }

    #[test]
    fn linked_worktree_shares_root_and_is_flagged() {
        let tmp = TempDir::new().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir(&main).unwrap();
        init_repo(&main);
        let wt = tmp.path().join("wt");
        git(&main, &["worktree", "add", "-q", wt.to_str().unwrap()]);

        let main_fp = capture(main.to_str().unwrap()).expect("main repo");
        let wt_fp = capture(wt.to_str().unwrap()).expect("worktree");
        assert!(!main_fp.is_worktree);
        assert!(wt_fp.is_worktree);
        assert_eq!(wt_fp.root_commit, main_fp.root_commit);
        // macOS TempDir paths may differ by /private symlink resolution;
        // compare canonicalized.
        let main_canon = std::fs::canonicalize(&main).unwrap();
        let reported = std::fs::canonicalize(wt_fp.main_path.expect("main path")).unwrap();
        assert_eq!(reported, main_canon);
    }

    #[test]
    fn non_git_dir_has_no_fingerprint() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(capture(tmp.path().to_str().unwrap()), None);
    }

    #[test]
    fn shallow_clone_omits_root_commit() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir(&src).unwrap();
        init_repo(&src);
        git(&src, &["commit", "--allow-empty", "-q", "-m", "two"]);

        let clone = tmp.path().join("clone");
        git(
            tmp.path(),
            &[
                "clone",
                "-q",
                "--no-local",
                "--depth",
                "1",
                src.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        // Give the clone a transferable remote (the file path one normalizes
        // to None) so the remote-only path is exercised too.
        git(
            &clone,
            &["remote", "set-url", "origin", "git@github.com:acme/foo.git"],
        );

        let fp = capture(clone.to_str().unwrap()).expect("is a repo");
        assert_eq!(
            fp.root_commit, None,
            "shallow root is a graft, not identity"
        );
        assert_eq!(fp.remote_url.as_deref(), Some("github.com/acme/foo"));
    }

    #[test]
    fn empty_repo_yields_empty_facts_not_none() {
        let tmp = TempDir::new().unwrap();
        git(tmp.path(), &["init", "-q", "-b", "main"]);

        let fp = capture(tmp.path().to_str().unwrap()).expect("is a repo");
        assert_eq!(fp.root_commit, None);
        assert_eq!(fp.remote_url, None);
    }

    #[test]
    fn missing_binary_and_timeout_degrade_to_none() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            run_with_timeout(
                "definitely-not-a-real-binary",
                tmp.path(),
                &["x"],
                Duration::from_millis(200)
            ),
            None
        );
        // `sleep 5` outlives a 100ms deadline → killed → None, promptly.
        let start = Instant::now();
        assert_eq!(
            run_with_timeout("/bin/sleep", tmp.path(), &["5"], Duration::from_millis(100)),
            None
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn cache_memoizes_per_path() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        let mut cache = FingerprintCache::default();
        let a = cache.get(tmp.path().to_str().unwrap());
        let b = cache.get(tmp.path().to_str().unwrap());
        assert_eq!(a, b);
        assert!(a.is_some());
    }
}
