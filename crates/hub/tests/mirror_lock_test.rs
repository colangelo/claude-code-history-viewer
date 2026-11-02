//! #105: a mirror file held open by ANOTHER process is locked, not unreadable.
//! Opening it must neither rename it aside nor create a replacement: that is
//! what made the live hub adopt an empty file and cold-build (prod 503s on
//! 2026-11-02, from canary hubs and from swaps that overlapped the old hub).
//!
//! DuckDB's lock is a POSIX file lock, which is per process, so the holder is
//! this same test binary re-run as a child (`hold_the_mirror_when_asked`).

use hub::config::MirrorConfig;
use hub::mirror::Mirror;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const HOLD_ENV: &str = "CCHV_TEST_HOLD_MIRROR";
const HOLD_SECS_ENV: &str = "CCHV_TEST_HOLD_SECS";

/// Child mode only: open the file named by `CCHV_TEST_HOLD_MIRROR`, signal
/// readiness, hold it for `CCHV_TEST_HOLD_SECS`. A no-op in a normal run.
#[test]
fn hold_the_mirror_when_asked() {
    let Ok(path) = std::env::var(HOLD_ENV) else {
        return;
    };
    let secs: u64 = std::env::var(HOLD_SECS_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let conn = duckdb::Connection::open(&path).expect("child opens the mirror");
    std::fs::write(format!("{path}.held"), "1").expect("signal ready");
    std::thread::sleep(Duration::from_secs(secs));
    drop(conn);
}

fn cfg(dir: &Path, lock_wait_secs: u64) -> MirrorConfig {
    MirrorConfig {
        path: Some(dir.join("stats.duckdb")),
        lock_wait_secs,
        ..MirrorConfig::default()
    }
}

fn hold(path: &Path, secs: u64) -> Child {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "hold_the_mirror_when_asked", "--test-threads=1"])
        .env(HOLD_ENV, path)
        .env(HOLD_SECS_ENV, secs.to_string())
        .spawn()
        .expect("spawn the holder");
    let ready = PathBuf::from(format!("{}.held", path.display()));
    let until = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        assert!(Instant::now() < until, "holder never got the file");
        std::thread::sleep(Duration::from_millis(50));
    }
    child
}

fn asides(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.contains(".aside-"))
        .collect()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cchv-mirlock-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_mirror_held_by_another_process_is_not_moved_aside() {
    let dir = fresh_dir("held");
    let c = cfg(&dir, 1);
    drop(Mirror::open_or_create(&c).expect("create the mirror"));
    let path = c.resolved_path();
    let inode = std::fs::metadata(&path).unwrap().ino();

    let mut holder = hold(&path, 20);
    let opened = Mirror::open_or_create(&c);
    holder.kill().ok();
    holder.wait().ok();

    assert!(
        opened.is_err(),
        "a locked mirror is an error, not a new file"
    );
    assert_eq!(
        asides(&dir),
        Vec::<String>::new(),
        "nothing was moved aside"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().ino(),
        inode,
        "the held file is still the one at the path"
    );
}

#[test]
fn the_open_waits_for_a_lock_that_is_released() {
    let dir = fresh_dir("wait");
    let c = cfg(&dir, 15);
    drop(Mirror::open_or_create(&c).expect("create the mirror"));
    let path = c.resolved_path();
    let inode = std::fs::metadata(&path).unwrap().ino();

    let mut holder = hold(&path, 2);
    let opened = Mirror::open_or_create(&c);
    holder.wait().ok();

    assert!(
        opened.is_ok(),
        "opened once the holder let go: {:?}",
        opened.err()
    );
    assert_eq!(asides(&dir), Vec::<String>::new());
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
}

/// The control: a file that is NOT a DuckDB database still goes aside and is
/// replaced, as before.
#[test]
fn an_unreadable_file_still_goes_aside() {
    let dir = fresh_dir("junk");
    let c = cfg(&dir, 1);
    std::fs::write(c.resolved_path(), b"this is not a duckdb file at all").unwrap();
    Mirror::open_or_create(&c).expect("replaced by a fresh mirror");
    assert_eq!(asides(&dir).len(), 1, "the junk file was preserved aside");
}
