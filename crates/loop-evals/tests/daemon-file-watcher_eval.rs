//! T2 evals for the sync-daemon debounced file-watcher (`sync_daemon::watcher`)
//! and its new config fields (`sync_daemon::config::DaemonConfig`).
//!
//! The watcher is a latency optimization only — correctness always comes
//! from the periodic rescan plus the hub's idempotent ingest (see the
//! `history-sync-daemon` spec, decision D6). These evals drive only the
//! frozen public stub surface committed ahead of implementation:
//! `sync_daemon::watcher::{spawn, PassThrottle}` and
//! `sync_daemon::config::DaemonConfig`. All filesystem activity happens
//! inside a fresh `tempfile::TempDir` per test — never against a real home
//! directory.
//!
//! The unmodified stub compiles against every test here but never signals
//! (`spawn` registers nothing) and `pass_due` is always `false`, and the
//! config watch fields default to `0` — so every criterion fails at runtime
//! today. All waits are wrapped in outer `tokio::time::timeout` bounds so a
//! stuck watcher fails the eval instead of hanging the suite.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use sync_daemon::config::DaemonConfig;
use sync_daemon::watcher::{spawn, PassThrottle};

/// Give the underlying OS watcher a moment to finish registering before we
/// generate filesystem events, so early events aren't missed by a watcher
/// that hasn't started listening yet.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

#[tokio::test]
async fn ac1_creating_and_writing_a_file_yields_a_signal() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let (_guard, mut rx) = spawn(&[dir.path().to_path_buf()], Duration::from_secs(1))
        .expect("spawn must succeed for a single valid root");

    settle().await;

    let file_path = dir.path().join("session.jsonl");
    std::fs::write(&file_path, b"{\"hello\":true}\n").expect("write file under watched root");

    let signal = tokio::time::timeout(Duration::from_secs(30), rx.recv()).await;
    assert!(
        matches!(signal, Ok(Some(()))),
        "expected a debounced signal within 30s after creating a file, got {signal:?}"
    );
}

#[tokio::test]
async fn ac2_new_subdirectory_created_after_spawn_is_watched_recursively() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let (_guard, mut rx) = spawn(&[dir.path().to_path_buf()], Duration::from_secs(1))
        .expect("spawn must succeed for a single valid root");

    settle().await;

    // A brand-new subdirectory (like a new provider project dir) created
    // after the watch was already registered.
    let sub = dir.path().join("new-project");
    std::fs::create_dir(&sub).expect("create new subdirectory");
    std::fs::write(sub.join("session.jsonl"), b"{}\n").expect("write file in new subdirectory");

    let signal = tokio::time::timeout(Duration::from_secs(30), rx.recv()).await;
    assert!(
        matches!(signal, Ok(Some(()))),
        "expected a signal for activity in a subdirectory created after spawn, within 30s, got {signal:?}"
    );
}

#[tokio::test]
async fn ac3_rapid_appends_produce_bounded_signal_count() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let (_guard, mut rx) = spawn(&[dir.path().to_path_buf()], Duration::from_secs(1))
        .expect("spawn must succeed for a single valid root");

    settle().await;

    let file_path = dir.path().join("watched.jsonl");
    std::fs::write(&file_path, b"").expect("create file to append to");

    // Count every signal delivered over a 10s window that starts now,
    // concurrently with the rapid appends below.
    let counter = tokio::spawn(async move {
        let mut count = 0u32;
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            while rx.recv().await.is_some() {
                count += 1;
            }
        })
        .await;
        count
    });

    for i in 0..30 {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&file_path)
            .expect("open watched file for append");
        writeln!(f, "line {i}").expect("append line");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let count = counter.await.expect("signal-counting task must not panic");
    assert!(
        (1..=6).contains(&count),
        "expected 1..=6 debounced signals from 30 rapid appends over a 10s window, got {count}"
    );
}

#[tokio::test]
async fn ac4_bad_root_before_valid_root_degrades_not_disables() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let bad_root = PathBuf::from("/this/path/does/not/exist/daemon-file-watcher-eval");
    let roots = vec![bad_root, dir.path().to_path_buf()];

    let (_guard, mut rx) = spawn(&roots, Duration::from_secs(1))
        .expect("spawn must return Ok even when an earlier root cannot be watched");

    settle().await;

    std::fs::write(dir.path().join("session.jsonl"), b"{}\n")
        .expect("write file under the valid root");

    let signal = tokio::time::timeout(Duration::from_secs(30), rx.recv()).await;
    assert!(
        matches!(signal, Ok(Some(()))),
        "expected a signal from the valid root within 30s despite a bad root earlier in the list, got {signal:?}"
    );
}

#[test]
fn ac5_pass_throttle_remembers_pending_trigger_across_the_gap() {
    let min_gap = Duration::from_secs(30);
    let mut throttle = PassThrottle::new(min_gap);

    let t0 = Instant::now();
    assert!(
        !throttle.pass_due(t0),
        "no trigger has ever been recorded -> not due"
    );

    throttle.note_trigger(t0);
    assert!(
        throttle.pass_due(t0),
        "a pending trigger with no prior pass must be due immediately"
    );

    throttle.note_pass(t0);
    throttle.note_trigger(t0 + Duration::from_secs(1));
    assert!(
        !throttle.pass_due(t0 + Duration::from_secs(1)),
        "a trigger arriving inside the min_gap window must not fire yet"
    );
    assert!(
        throttle.pass_due(t0 + Duration::from_secs(31)),
        "the pending trigger from inside the gap must survive and fire once the gap elapses"
    );
    assert!(
        !throttle.pass_due(t0 + Duration::from_secs(32)),
        "the trigger must be consumed once pass_due returns true for it"
    );
}

#[test]
fn ac6_config_watch_fields_default_and_honor_explicit_values() {
    let without_watch_fields = r#"
        hub_url = "http://hub.tailnet:8787"
        hub_token = "tok-123"
    "#;
    let cfg: DaemonConfig =
        toml::from_str(without_watch_fields).expect("parse config without watch fields");
    assert_eq!(
        cfg.watch_debounce_secs, 2,
        "default watch_debounce_secs must be 2"
    );
    assert_eq!(
        cfg.watch_min_pass_gap_secs, 30,
        "default watch_min_pass_gap_secs must be 30"
    );

    let with_watch_fields = r#"
        hub_url = "http://hub.tailnet:8787"
        hub_token = "tok-123"
        watch_debounce_secs = 5
        watch_min_pass_gap_secs = 120
    "#;
    let cfg: DaemonConfig =
        toml::from_str(with_watch_fields).expect("parse config with explicit watch fields");
    assert_eq!(
        cfg.watch_debounce_secs, 5,
        "explicit watch_debounce_secs must be honored"
    );
    assert_eq!(
        cfg.watch_min_pass_gap_secs, 120,
        "explicit watch_min_pass_gap_secs must be honored"
    );
}
