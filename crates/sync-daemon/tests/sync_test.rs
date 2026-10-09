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
    /// `Some(n)`: accept the next `n` batches, then fail every later one.
    fail_after: Option<usize>,
}

impl HubClient for MockHub {
    // Not an `async fn`: this mock does no I/O, so it hands back an
    // already-complete future rather than one that never awaits
    // (`clippy::unused_async_trait_impl`, `-D warnings` in CI). The real client
    // in `client.rs` stays async.
    fn ingest(
        &self,
        batch: &IngestBatch,
    ) -> impl std::future::Future<Output = anyhow::Result<IngestResponse>> {
        std::future::ready(self.record(batch))
    }
}

impl MockHub {
    /// The mock's whole behaviour, kept synchronous and readable.
    fn record(&self, batch: &IngestBatch) -> anyhow::Result<IngestResponse> {
        let mut s = self.state.lock().unwrap();
        if s.fail_remaining > 0 {
            s.fail_remaining -= 1;
            anyhow::bail!("simulated hub failure");
        }
        match s.fail_after {
            Some(0) => anyhow::bail!("simulated hub failure mid-send"),
            Some(n) => s.fail_after = Some(n - 1),
            None => {}
        }
        s.batches.push(batch.clone());
        Ok(IngestResponse::default())
    }

    fn fail_next(&self, n: usize) {
        self.state.lock().unwrap().fail_remaining = n;
    }
    fn fail_after(&self, successes: Option<usize>) {
        self.state.lock().unwrap().fail_after = successes;
    }
    /// Every message the hub received, in arrival order, as `(seq, key)`.
    fn sent(&self) -> Vec<(i32, String)> {
        self.state
            .lock()
            .unwrap()
            .batches
            .iter()
            .flat_map(|b| b.messages.iter().map(|m| (m.seq, m.message_key.clone())))
            .collect()
    }
    /// Sessions carried by every batch, so a tail-only pass can be checked to
    /// still refresh the session row.
    fn session_rows(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .batches
            .iter()
            .map(|b| b.sessions.len())
            .sum()
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
async fn a_session_failing_every_pass_is_deferred_then_retried_when_it_changes() {
    let fx = fixture();
    let file = two_message_session(&fx.home);

    let hub = MockHub::default();
    hub.fail_next(usize::MAX); // this session can never be delivered
    let mut cp = Checkpoint::load(&fx.state_dir);

    // The grace window retries at full cost, once per pass.
    for pass in 1..=3 {
        let stats = sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
        assert_eq!(stats.errors, 1, "pass {pass} must still attempt delivery");
        assert_eq!(
            stats.sessions_deferred, 0,
            "pass {pass} is inside the grace window"
        );
    }

    // Past it, the session is skipped without an attempt.
    let stats = sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(stats.errors, 0, "a deferred session is not an error");
    assert_eq!(
        stats.sessions_deferred, 1,
        "expected the session to back off"
    );

    // Touching the file resets the streak: it is attempted again immediately,
    // and once the hub recovers it delivers and the failure record is cleared.
    let mut content = std::fs::read_to_string(&file).unwrap();
    content.push_str(&format!(
        "{}\n",
        user_line(
            "u3",
            "sess-1",
            "2026-01-02T00:00:00Z",
            "a third message",
            "/Users/test/proj"
        )
    ));
    std::fs::write(&file, content).unwrap();
    hub.fail_next(0);
    let stats = sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(
        stats.sessions_deferred, 0,
        "an edited file must be retried now"
    );
    assert!(stats.sessions_synced >= 1);
    assert!(
        cp.failures.is_empty(),
        "a success clears the failure streak"
    );
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

/// Claude Code writes state records (`permission-mode`, `custom-title`, `mode`, …)
/// with no `uuid` and no `timestamp`. history-core fills both at parse time, the
/// timestamp with `now()`. While `message_key` hashed that timestamp, every
/// re-parse of a growing file re-keyed every such record, and the hub stored
/// each one again. Measured 2026-09-23: 17.96M of 19.32M archived rows were
/// these copies, collapsing to 165k real records. A re-parse must re-send
/// the same key for the same record.
#[tokio::test]
#[serial]
async fn a_record_without_uuid_or_timestamp_keeps_its_key_across_reparses() {
    let fx = fixture();
    let file = two_message_session(&fx.home);
    let mut content = std::fs::read_to_string(&file).unwrap();
    content.push_str(
        "{\"type\":\"permission-mode\",\"permissionMode\":\"bypassPermissions\",\"sessionId\":\"sess-1\"}\n",
    );
    std::fs::write(&file, &content).unwrap();

    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    let before = hub.message_keys();
    assert_eq!(before.len(), 3, "two turns plus the state record");

    // The file grows, so the whole session is parsed again.
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
    std::fs::write(&file, &content).unwrap();
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;

    // The mock keeps every key it ever received, so a re-keyed record shows up
    // as a fifth key rather than as a missing one.
    let after = hub.message_keys();
    assert_eq!(after.len(), 4, "exactly one new key: the appended turn");
}

// ----- #49: send only the tail ----------------------------------------------
//
// A changed session used to be re-sent in full on every pass, and the hub threw
// the already-archived rows away (`ON CONFLICT DO NOTHING`). Measured 2026-10-09:
// ~522k messages sent in 10 minutes for ~1k new ones, a steady ~4.5 MB/s from
// the hub to pg1. A pass now sends only what follows the last acknowledged
// message, and falls back to a full re-send whenever the file is not a plain
// append of what was sent.

/// `n` user turns with distinct uuids and timestamps.
fn turns(sid: &str, range: std::ops::Range<usize>) -> Vec<String> {
    range
        .map(|i| {
            user_line(
                &format!("u{i}"),
                sid,
                &format!("2026-01-01T00:{:02}:00Z", i % 60),
                &format!("turn number {i}"),
                "/Users/test/proj",
            )
        })
        .collect()
}

fn session_with(home: &Path, lines: &[String]) -> PathBuf {
    write_session(home, "-Users-test-proj", "sess-1", lines)
}

fn append(file: &Path, lines: &[String]) {
    let mut content = std::fs::read_to_string(file).unwrap();
    for l in lines {
        content.push_str(l);
        content.push('\n');
    }
    std::fs::write(file, content).unwrap();
}

/// The keys a fresh daemon (empty checkpoint) sends for the file as it is now:
/// what a full re-parse produces, to compare an incremental history against.
async fn full_parse_keys(fx: &Fixture) -> HashSet<String> {
    let hub = MockHub::default();
    let mut cp = Checkpoint::default();
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    hub.message_keys()
}

#[tokio::test]
#[serial]
async fn an_appended_session_sends_only_the_new_tail() {
    let fx = fixture();
    let file = session_with(&fx.home, &turns("sess-1", 0..5));
    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(hub.sent().len(), 5);

    append(&file, &turns("sess-1", 5..7));
    let rows_before = hub.session_rows();
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;

    let sent = hub.sent();
    let tail: Vec<i32> = sent[5..].iter().map(|(seq, _)| *seq).collect();
    assert_eq!(
        tail,
        vec![5, 6],
        "only the two appended turns, at their absolute seq"
    );
    assert!(
        hub.session_rows() > rows_before,
        "the session row is still refreshed"
    );
    assert_eq!(
        hub.message_keys(),
        full_parse_keys(&fx).await,
        "tail keys equal the keys a full re-parse produces"
    );
}

#[tokio::test]
#[serial]
async fn an_unchanged_append_count_sends_no_messages_but_refreshes_the_session() {
    // The file changed (mtime/size) but parses to the same messages — e.g. a
    // trailing partial line, or the stat raced an append that the parse already
    // saw. Nothing is re-sent; the session row still goes.
    let fx = fixture();
    session_with(&fx.home, &turns("sess-1", 0..3));
    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;

    // Pretend the checkpoint's stat predates the parse it recorded.
    for st in cp.files.values_mut() {
        st.size -= 1;
    }
    let rows_before = hub.session_rows();
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(hub.sent().len(), 3, "no message re-sent");
    assert!(
        hub.session_rows() > rows_before,
        "the session row is still refreshed"
    );
}

#[tokio::test]
#[serial]
async fn a_shrunk_file_is_resent_in_full() {
    let fx = fixture();
    let file = session_with(&fx.home, &turns("sess-1", 0..4));
    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;

    // Truncated to two turns, then one new turn: fewer bytes than before.
    std::fs::write(&file, "").unwrap();
    append(&file, &turns("sess-1", 0..2));
    append(&file, &turns("sess-1", 9..10));
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    let pass2: Vec<i32> = hub.sent()[4..].iter().map(|(s, _)| *s).collect();
    assert_eq!(
        pass2,
        vec![0, 1, 2],
        "a shrink re-sends everything from seq 0"
    );

    // And the next append is a tail again.
    append(&file, &turns("sess-1", 10..11));
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    let pass3: Vec<i32> = hub.sent()[7..].iter().map(|(s, _)| *s).collect();
    assert_eq!(pass3, vec![3]);
}

#[tokio::test]
#[serial]
async fn a_rewritten_prefix_is_resent_in_full() {
    // Same or larger size, more messages, but an EARLIER message changed: a
    // naive "send past message_count" would silently miss the rewrite.
    let fx = fixture();
    let file = session_with(&fx.home, &turns("sess-1", 0..3));
    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;

    let mut lines = turns("sess-1", 0..4);
    lines[1] = user_line(
        "u1",
        "sess-1",
        "2026-01-01T00:01:00Z",
        "turn number 1, edited in place with a longer text",
        "/Users/test/proj",
    );
    std::fs::write(&file, format!("{}\n", lines.join("\n"))).unwrap();
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;

    let pass2: Vec<i32> = hub.sent()[3..].iter().map(|(s, _)| *s).collect();
    assert_eq!(
        pass2,
        vec![0, 1, 2, 3],
        "a rewritten prefix re-sends the whole session"
    );
    assert!(
        full_parse_keys(&fx).await.is_subset(&hub.message_keys()),
        "the edited turn's new key reached the hub"
    );
}

#[tokio::test]
#[serial]
async fn a_send_interrupted_mid_tail_resumes_from_the_last_acknowledged_pass() {
    let fx = fixture();
    let file = session_with(&fx.home, &turns("sess-1", 0..2));
    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;

    // Three new turns sent one per batch; the hub dies after the first batch.
    append(&file, &turns("sess-1", 2..5));
    hub.fail_after(Some(1));
    let stats = sync::run_once(&hub, &fx.identity, &mut cp, 1, &[]).await;
    assert!(stats.errors >= 1);

    // Restart: the checkpoint on disk still says "two messages acknowledged".
    hub.fail_after(None);
    let mut cp2 = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp2, 1, &[]).await;

    let after_restart: Vec<i32> = hub.sent()[3..].iter().map(|(s, _)| *s).collect();
    assert_eq!(
        after_restart,
        vec![2, 3, 4],
        "resend starts at the last acknowledged count, not at 0"
    );
    assert_eq!(
        hub.message_keys(),
        full_parse_keys(&fx).await,
        "nothing missing"
    );
}

#[tokio::test]
#[serial]
async fn a_checkpoint_from_before_tail_sync_resends_once_then_tails() {
    let fx = fixture();
    let file = session_with(&fx.home, &turns("sess-1", 0..3));
    let hub = MockHub::default();
    let mut cp = Checkpoint::load(&fx.state_dir);
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;

    // An entry written by an older daemon has no prefix digest.
    for st in cp.files.values_mut() {
        st.prefix_digest = None;
    }
    append(&file, &turns("sess-1", 3..4));
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(
        hub.sent().len(),
        3 + 4,
        "no digest to trust: one full re-send"
    );

    append(&file, &turns("sess-1", 4..5));
    sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]).await;
    assert_eq!(hub.sent().len(), 3 + 4 + 1, "then tails again");
}

#[test]
fn a_legacy_checkpoint_json_still_loads_without_a_digest() {
    let json = r#"{"files":{"f":{"size":1,"mtime_ms":2,"message_count":3,"last_synced_ms":4}}}"#;
    let c: Checkpoint = serde_json::from_str(json).expect("legacy checkpoint must parse");
    assert_eq!(c.files["f"].prefix_digest, None);
}
