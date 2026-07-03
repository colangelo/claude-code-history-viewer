//! Integration tests for the sync daemon.
//!
//! Each test points `$HOME` at a temp dir containing a fake `~/.claude` fixture
//! and runs the real history-core enumeration against a mock hub. `$HOME` is
//! process-global, so every test is `#[serial]` and the suite must run with
//! `--test-threads=1` as well (matching the repo convention).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use archive_protocol::{IngestBatch, IngestResponse};
use serde_json::json;
use serial_test::serial;
use sync_daemon::checkpoint::Checkpoint;
use sync_daemon::client::HubClient;
use sync_daemon::identity::Identity;
use sync_daemon::sync;
use tempfile::TempDir;

// ----- mock hub -----------------------------------------------------------

#[derive(Clone, Default)]
struct MockHub {
    state: Arc<Mutex<MockState>>,
}

#[derive(Default)]
struct MockState {
    batches: Vec<IngestBatch>,
    fail_remaining: usize,
}

impl HubClient for MockHub {
    async fn ingest(&self, batch: &IngestBatch) -> anyhow::Result<IngestResponse> {
        let mut s = self.state.lock().unwrap();
        if s.fail_remaining > 0 {
            s.fail_remaining -= 1;
            anyhow::bail!("simulated hub failure");
        }
        s.batches.push(batch.clone());
        Ok(IngestResponse::default())
    }
}

impl MockHub {
    fn fail_next(&self, n: usize) {
        self.state.lock().unwrap().fail_remaining = n;
    }
    fn total_messages(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .batches
            .iter()
            .map(|b| b.messages.len())
            .sum()
    }
    fn message_keys(&self) -> HashSet<String> {
        self.state
            .lock()
            .unwrap()
            .batches
            .iter()
            .flat_map(|b| b.messages.iter().map(|m| m.message_key.clone()))
            .collect()
    }
    fn search_texts(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .batches
            .iter()
            .flat_map(|b| b.messages.iter().filter_map(|m| m.search_text.clone()))
            .collect()
    }
}

// ----- fixture ------------------------------------------------------------

struct Fixture {
    _home: TempDir,
    home: PathBuf,
    identity: Identity,
    state_dir: PathBuf,
}

fn fixture() -> Fixture {
    let home = TempDir::new().unwrap();
    std::env::set_var("HOME", home.path());
    let state_dir = home.path().join("sync-state");
    let identity = Identity::load_or_create(&state_dir).unwrap();
    Fixture {
        home: home.path().to_path_buf(),
        _home: home,
        identity,
        state_dir,
    }
}

fn user_line(uuid: &str, sid: &str, ts: &str, text: &str, cwd: &str) -> String {
    json!({
        "uuid": uuid, "sessionId": sid, "timestamp": ts, "type": "user", "cwd": cwd,
        "message": { "role": "user", "content": text }
    })
    .to_string()
}

fn assistant_line(uuid: &str, parent: &str, sid: &str, ts: &str, text: &str) -> String {
    json!({
        "uuid": uuid, "parentUuid": parent, "sessionId": sid, "timestamp": ts, "type": "assistant",
        "message": { "role": "assistant", "model": "claude-x", "content": [{ "type": "text", "text": text }] }
    })
    .to_string()
}

fn write_session(home: &Path, project_dir: &str, session: &str, lines: &[String]) -> PathBuf {
    let dir = home.join(".claude/projects").join(project_dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{session}.jsonl"));
    std::fs::write(&file, format!("{}\n", lines.join("\n"))).unwrap();
    file
}

fn two_message_session(home: &Path) -> PathBuf {
    write_session(
        home,
        "-Users-test-proj",
        "sess-1",
        &[
            user_line(
                "u1",
                "sess-1",
                "2026-01-01T00:00:00Z",
                "hello quick fox",
                "/Users/test/proj",
            ),
            assistant_line(
                "u2",
                "u1",
                "sess-1",
                "2026-01-01T00:01:00Z",
                "hi there friend",
            ),
        ],
    )
}

// ----- tests --------------------------------------------------------------

#[tokio::test]
#[serial]
async fn cold_start_delivers_everything_once() {
    let fx = fixture();
    two_message_session(&fx.home);
    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);

    let stats = sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;

    assert!(stats.sessions_synced >= 1, "synced a session");
    assert_eq!(hub.total_messages(), 2, "both messages delivered");
    assert!(!cp.files.is_empty(), "checkpoint recorded the file");
}

#[tokio::test]
#[serial]
async fn checkpoint_survives_restart_no_redundant_delivery() {
    let fx = fixture();
    two_message_session(&fx.home);

    let hub1 = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub1, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(hub1.total_messages(), 2);

    // Simulate a restart: reload the checkpoint from disk, fresh hub.
    let hub2 = MockHub::default();
    let mut cp2 = Checkpoint::load(&fx.state_dir);
    let stats = sync::run_once(&hub2, &fx.identity, &mut cp2, 500, &[]).await;
    assert_eq!(
        hub2.total_messages(),
        0,
        "unchanged session not re-delivered"
    );
    assert!(stats.sessions_skipped >= 1);
}

#[tokio::test]
#[serial]
async fn appended_messages_sync_on_next_pass() {
    let fx = fixture();
    let file = two_message_session(&fx.home);

    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    let before = hub.message_keys();
    assert_eq!(before.len(), 2);

    // Append a third message (file size grows → change detected).
    let mut content = std::fs::read_to_string(&file).unwrap();
    content.push_str(&format!(
        "{}\n",
        user_line(
            "u3",
            "sess-1",
            "2026-01-02T00:00:00Z",
            "a third turtle message",
            "/Users/test/proj"
        )
    ));
    std::fs::write(&file, content).unwrap();

    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    let after = hub.message_keys();
    assert_eq!(after.len(), 3, "the appended message's key is new");
    assert!(before.is_subset(&after));
}

#[tokio::test]
#[serial]
async fn failed_delivery_is_not_checkpointed_and_resends() {
    let fx = fixture();
    two_message_session(&fx.home);

    let hub = MockHub::default();
    hub.fail_next(1); // first ingest call fails
    let mut cp = Checkpoint::load(&fx.state_dir);

    let stats1 = sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(
        hub.total_messages(),
        0,
        "nothing delivered when ingest failed"
    );
    assert!(stats1.errors >= 1);
    assert!(cp.files.is_empty(), "checkpoint not advanced on failure");

    // Next pass (safety-net rescan) succeeds — at-least-once delivery.
    let stats2 = sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(hub.total_messages(), 2, "redelivered on the next pass");
    assert!(stats2.sessions_synced >= 1);
}

#[tokio::test]
#[serial]
async fn machine_id_is_stable_across_restarts() {
    let fx = fixture();
    let id1 = Identity::load_or_create(&fx.state_dir).unwrap().machine_id;
    let id2 = Identity::load_or_create(&fx.state_dir).unwrap().machine_id;
    assert_eq!(id1, id2, "machine id persists across loads");
    assert_eq!(id1, fx.identity.machine_id);
}

#[tokio::test]
#[serial]
async fn deleted_source_leaves_archive_intact() {
    let fx = fixture();
    let file = two_message_session(&fx.home);

    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(hub.total_messages(), 2);

    // Delete the local source: the daemon must NOT issue any delete.
    std::fs::remove_file(&file).unwrap();
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(
        hub.total_messages(),
        2,
        "archive unchanged — local deletion never removes hub rows"
    );
}

#[tokio::test]
#[serial]
async fn search_text_is_computed_and_delivered() {
    let fx = fixture();
    two_message_session(&fx.home);
    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;

    let texts = hub.search_texts();
    assert!(
        texts.iter().any(|t| t.contains("fox")),
        "flattened search_text reaches the wire: {texts:?}"
    );
}

#[tokio::test]
#[serial]
async fn config_loads_from_url_and_token_without_db() {
    std::env::remove_var("DAEMON_CONFIG");
    std::env::set_var("HUB_URL", "http://hub.example:8787");
    std::env::set_var("HUB_TOKEN", "secret");
    let cfg = sync_daemon::config::DaemonConfig::load().unwrap();
    assert_eq!(cfg.hub_url, "http://hub.example:8787");
    assert_eq!(cfg.hub_token, "secret");
    // There is no database field on DaemonConfig — daemons never hold DB creds.
}
