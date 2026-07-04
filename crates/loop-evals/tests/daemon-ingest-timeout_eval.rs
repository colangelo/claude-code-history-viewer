//! T2 evals for the sync-daemon ingest-timeout fix (Gitea issue #2): the
//! ingest HTTP client (`ReqwestHubClient`, `crates/sync-daemon/src/client.rs`)
//! is built from `reqwest::Client::new()` with no timeout of any kind, so a
//! request straddling e.g. a laptop sleep cycle can block `send()` forever and
//! wedge the whole daemon `run()` loop indefinitely. Two independent, layered
//! fixes are under test:
//!
//! - a per-request `reqwest` timeout, configured via `CCHV_INGEST_TIMEOUT_SECS`
//!   (AC1/AC2, exercised directly against `ReqwestHubClient::ingest`)
//! - a per-batch deadline at the sync layer, configured via
//!   `CCHV_INGEST_DEADLINE_SECS`, that bounds `sync::run_once` even if a
//!   `HubClient` implementation hangs for any reason (AC3/AC4)
//!
//! Every eval wraps the operation under test in an outer `tokio::time::timeout`
//! and fails (does not hang) if it fires — that outer bound is what makes each
//! eval fail against the unmodified crate, where the operation blocks forever.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use archive_protocol::{IngestBatch, IngestResponse, MachineInfo};
use serde_json::json;
use serial_test::serial;
use sync_daemon::checkpoint::Checkpoint;
use sync_daemon::client::{HubClient, ReqwestHubClient};
use sync_daemon::identity::Identity;
use sync_daemon::sync;
use tempfile::TempDir;
use tokio::net::TcpListener;
use uuid::Uuid;

/// Outer bound for the timeout-path evals (AC1/AC2) — generous so the fixed
/// client passes with margin; on the unmodified client this fires instead of
/// letting the test hang forever.
const OUTER_BOUND_TIMEOUT: Duration = Duration::from_secs(120);
/// Outer bound for the deadline-path evals (AC3/AC4).
const OUTER_BOUND_DEADLINE: Duration = Duration::from_secs(60);

// ----- black-hole hub (AC1/AC2) -------------------------------------------

/// Accepts TCP connections and holds them open forever without ever writing a
/// response. Returns the `http://` base URL to POST against and a shared
/// counter of accepted connections.
async fn spawn_black_hole() -> (String, Arc<Mutex<usize>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(Mutex::new(0usize));
    let accepted_task = accepted.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            *accepted_task.lock().unwrap() += 1;
            held.push(stream); // never read/write — the connection just hangs
        }
    });
    (format!("http://{addr}"), accepted)
}

fn empty_batch(machine_id: Uuid) -> IngestBatch {
    IngestBatch {
        machine: MachineInfo {
            machine_id,
            hostname: "black-hole-eval".into(),
            os: Some("test".into()),
        },
        projects: vec![],
        sessions: vec![],
        messages: vec![],
    }
}

// ----- hanging HubClient (AC3/AC4) ----------------------------------------

/// A `HubClient` whose `ingest` never resolves — stands in for any
/// `HubClient` implementation hanging for any reason, which the per-batch
/// deadline in `sync::run_once` must bound regardless.
struct HangingHub;

impl HubClient for HangingHub {
    async fn ingest(&self, _batch: &IngestBatch) -> anyhow::Result<IngestResponse> {
        std::future::pending::<()>().await;
        unreachable!("this future never resolves")
    }
}

/// A `HubClient` that records every batch it receives and always succeeds.
#[derive(Clone, Default)]
struct RecordingHub {
    batches: Arc<Mutex<Vec<IngestBatch>>>,
}

impl HubClient for RecordingHub {
    async fn ingest(&self, batch: &IngestBatch) -> anyhow::Result<IngestResponse> {
        self.batches.lock().unwrap().push(batch.clone());
        Ok(IngestResponse::default())
    }
}

impl RecordingHub {
    fn total_messages(&self) -> usize {
        self.batches
            .lock()
            .unwrap()
            .iter()
            .map(|b| b.messages.len())
            .sum()
    }
}

// ----- $HOME session fixture (AC3/AC4) — shape copied from
// crates/sync-daemon/tests/sync_test.rs -------------------------------------

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

// ----- AC1/AC2: per-request timeout unwedges a stuck send() ---------------

#[tokio::test]
#[serial]
async fn ac1_timed_out_request_returns_err_instead_of_hanging_forever() {
    std::env::set_var("CCHV_INGEST_TIMEOUT_SECS", "1");
    let (base_url, _accepted) = spawn_black_hole().await;
    let client = ReqwestHubClient::new(&base_url, "tok");
    let batch = empty_batch(Uuid::new_v4());

    let result = tokio::time::timeout(OUTER_BOUND_TIMEOUT, client.ingest(&batch)).await;
    std::env::remove_var("CCHV_INGEST_TIMEOUT_SECS");

    let ingest_result = result.expect(
        "ingest() must return within the 120s outer bound instead of hanging forever \
         on a black-hole hub — this requires CCHV_INGEST_TIMEOUT_SECS to actually bound \
         the underlying reqwest::Client",
    );
    assert!(
        ingest_result.is_err(),
        "a black-hole hub can never succeed — ingest() must eventually yield Err"
    );
}

#[tokio::test]
#[serial]
async fn ac2_timed_out_requests_go_through_the_existing_retry_loop() {
    std::env::set_var("CCHV_INGEST_TIMEOUT_SECS", "1");
    let (base_url, accepted) = spawn_black_hole().await;
    let client = ReqwestHubClient::new(&base_url, "tok");
    let batch = empty_batch(Uuid::new_v4());

    let result = tokio::time::timeout(OUTER_BOUND_TIMEOUT, client.ingest(&batch)).await;
    std::env::remove_var("CCHV_INGEST_TIMEOUT_SECS");

    let _ = result.expect("ingest() must return within the 120s outer bound, not hang forever");
    let connections = *accepted.lock().unwrap();
    assert!(
        connections >= 2,
        "expected at least 2 accepted connections (proving the retry/backoff loop ran \
         multiple attempts rather than failing single-shot), got {connections}"
    );
}

// ----- AC3/AC4: per-batch deadline bounds a hanging HubClient --------------

#[tokio::test]
#[serial]
async fn ac3_deadline_aborts_hanging_ingest_and_counts_an_error() {
    std::env::set_var("CCHV_INGEST_DEADLINE_SECS", "2");
    let fx = fixture();
    two_message_session(&fx.home);
    let hub = HangingHub;
    let mut cp = Checkpoint::load(&fx.state_dir);

    let stats = tokio::time::timeout(
        OUTER_BOUND_DEADLINE,
        sync::run_once(&hub, &fx.identity, &mut cp, 500, &[]),
    )
    .await;
    std::env::remove_var("CCHV_INGEST_DEADLINE_SECS");

    let stats = stats.expect(
        "run_once() must return within the 60s outer bound instead of hanging forever \
         on a HubClient whose ingest() never resolves — this requires \
         CCHV_INGEST_DEADLINE_SECS to actually bound each client.ingest() call",
    );
    assert!(
        stats.errors >= 1,
        "the deadline-aborted batch must be counted in SyncStats::errors, got {stats:?}"
    );
}

#[tokio::test]
#[serial]
async fn ac4_aborted_batch_is_not_checkpointed_and_redelivers_next_pass() {
    std::env::set_var("CCHV_INGEST_DEADLINE_SECS", "2");
    let fx = fixture();
    two_message_session(&fx.home);
    let hanging = HangingHub;
    let mut cp = Checkpoint::load(&fx.state_dir);

    let stats1 = tokio::time::timeout(
        OUTER_BOUND_DEADLINE,
        sync::run_once(&hanging, &fx.identity, &mut cp, 500, &[]),
    )
    .await
    .expect("first run_once() must not hang forever on a hanging HubClient");
    assert!(
        stats1.errors >= 1,
        "the hanging ingest must count as an error"
    );
    assert!(
        cp.files.is_empty(),
        "the deadline-aborted batch must NOT be checkpointed — it must be retried \
         from the same position on the next pass"
    );

    let recorder = RecordingHub::default();
    let stats2 = tokio::time::timeout(
        OUTER_BOUND_DEADLINE,
        sync::run_once(&recorder, &fx.identity, &mut cp, 500, &[]),
    )
    .await
    .expect("second run_once() must not hang");
    std::env::remove_var("CCHV_INGEST_DEADLINE_SECS");

    assert_eq!(
        recorder.total_messages(),
        2,
        "the previously aborted session's messages must be delivered on the retry pass \
         — at-least-once delivery, no data lost"
    );
    assert!(stats2.sessions_synced >= 1);
}
