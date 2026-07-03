//! T2 evals for `GET /v1/healthz/ingest` — per-machine daemon-liveness
//! endpoint so Gatus (HTTP status/body only) can alert on a dead ingest daemon
//! even while `/v1/healthz` stays green (the incident: a daemon killed by
//! codesigning kept `/v1/healthz` happy while ~43k messages went unsynced).
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).
//! Each test seeds its own machine(s) via `POST /v1/ingest` with fresh random
//! `machine_id`s, following the existing test-isolation pattern (see
//! `crates/hub/tests/ingest_test.rs`). The one permitted raw-SQL exception is
//! backdating `machines.last_seen` to simulate a dead daemon deterministically.
//!
//! Shared-db caveat: `machines` accumulates forever across every suite/run on
//! this test database and is never truncated — as of writing it already holds
//! machines whose `last_seen` predates the default 7200s threshold. That makes
//! the endpoint's *global* ok/stale verdict impossible to pin to "ok" under
//! the *default* threshold: some historical machine is always stale. AC2/AC3
//! are unaffected (they only assert on individual machine entries, or
//! deliberately induce staleness themselves, so pre-existing pollution can
//! only agree with the assertion). AC1/AC4 need the endpoint to answer "ok"
//! globally, so those two tests pass a `stale_after_secs` far larger than any
//! plausible accumulated history — this exercises the identical ok/stale
//! computation the default threshold would, just at a value the shared db's
//! pollution can never cross.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use chrono::DateTime;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use uuid::Uuid;

/// Larger than any plausible accumulated test-db history (~31 years in
/// seconds) — see module doc.
const HUGE_STALE_AFTER_SECS: i64 = 999_999_999;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests")
}

struct TestHub {
    base: String,
    pool: PgPool,
}

fn token_for(machine_id: Uuid) -> String {
    format!("tok-{machine_id}")
}

async fn spawn(machine_ids: &[Uuid]) -> TestHub {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");

    let mut tokens = HashMap::new();
    for id in machine_ids {
        tokens.insert(token_for(*id), *id);
    }

    let state = hub::AppState {
        pool: pool.clone(),
        tokens: Arc::new(tokens),
    };
    let app = hub::router(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestHub {
        base: format!("http://{addr}"),
        pool,
    }
}

/// One project + one session + the given messages for `machine_id`.
fn sample_batch(
    machine_id: Uuid,
    hostname: &str,
    session: &str,
    messages: Vec<IngestMessage>,
) -> IngestBatch {
    IngestBatch {
        machine: MachineInfo {
            machine_id,
            hostname: hostname.into(),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: "/tmp/proj".into(),
            name: Some("proj".into()),
            storage_type: Some("jsonl".into()),
            session_count: Some(1),
            message_count: Some(i32::try_from(messages.len()).unwrap_or(0)),
            last_modified: None,
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: session.into(),
            project_path: Some("/tmp/proj".into()),
            file_path: Some(format!("/tmp/proj/{session}.jsonl")),
            entrypoint: None,
            summary: Some("a session".into()),
            message_count: Some(i32::try_from(messages.len()).unwrap_or(0)),
            first_message_time: None,
            last_message_time: None,
            last_modified: None,
            has_tool_use: Some(false),
            has_errors: Some(false),
            storage_type: Some("jsonl".into()),
        }],
        messages,
    }
}

fn msg(session: &str, key: &str, uuid: Option<&str>, ts: &str, text: &str) -> IngestMessage {
    IngestMessage {
        provider: "claude".into(),
        session_id: session.into(),
        message_key: key.into(),
        uuid: uuid.map(Into::into),
        parent_uuid: None,
        seq: 0,
        timestamp: Some(ts.into()),
        message_type: Some("user".into()),
        role: Some("user".into()),
        model: None,
        stop_reason: None,
        input_tokens: None,
        output_tokens: None,
        cache_creation_tokens: None,
        cache_read_tokens: None,
        cost_usd: None,
        duration_ms: None,
        is_sidechain: false,
        content: Some(json!([{ "type": "text", "text": text }])),
        raw: json!({ "uuid": uuid, "text": text, "orig": true }),
        search_text: Some(text.into()),
    }
}

async fn ingest(hub: &TestHub, machine_id: Uuid, batch: &IngestBatch) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(token_for(machine_id))
        .json(batch)
        .send()
        .await
        .expect("send ingest request")
}

/// Simulates a daemon killed hours ago — the one permitted raw-SQL exception.
async fn backdate_3h(hub: &TestHub, machine_id: Uuid) {
    sqlx::query("UPDATE machines SET last_seen = now() - interval '3 hours' WHERE machine_id = $1")
        .bind(machine_id)
        .execute(&hub.pool)
        .await
        .expect("backdate last_seen");
}

/// GETs `/v1/healthz/ingest`, deliberately without an `Authorization` header
/// (AC5), optionally with a raw query string (e.g. `"stale_after_secs=60"`).
async fn get_ingest_health(hub: &TestHub, query: Option<&str>) -> reqwest::Response {
    let url = match query {
        Some(q) => format!("{}/v1/healthz/ingest?{q}", hub.base),
        None => format!("{}/v1/healthz/ingest", hub.base),
    };
    reqwest::Client::new()
        .get(url)
        .send()
        .await
        .expect("send healthz/ingest request")
}

fn find_machine(body: &Value, machine_id: Uuid) -> &Value {
    body["machines"]
        .as_array()
        .expect("machines must be an array")
        .iter()
        .find(|m| m["machine_id"].as_str() == Some(machine_id.to_string().as_str()))
        .unwrap_or_else(|| panic!("machine {machine_id} missing from machines array"))
}

fn assert_rfc3339(entry: &Value, field: &str) {
    let s = entry[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} must be a string, got {:?}", entry[field]));
    DateTime::parse_from_rfc3339(s)
        .unwrap_or_else(|e| panic!("{field} = {s:?} is not RFC3339: {e}"));
}

#[tokio::test]
async fn ac1_fresh_machine_is_ok_with_full_fields() {
    let machine_id = Uuid::new_v4();
    let hub = spawn(&[machine_id]).await;
    let batch = sample_batch(
        machine_id,
        "host-ac1",
        "sess-ac1",
        vec![msg(
            "sess-ac1",
            "k1",
            Some("u1"),
            "2026-01-01T00:00:00Z",
            "hello",
        )],
    );
    assert_eq!(ingest(&hub, machine_id, &batch).await.status(), 200);

    // Huge threshold escapes shared-db pollution for the overall verdict —
    // see module doc.
    let resp = get_ingest_health(
        &hub,
        Some(&format!("stale_after_secs={HUGE_STALE_AFTER_SECS}")),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.expect("parse json body");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["stale_after_secs"], HUGE_STALE_AFTER_SECS);

    let entry = find_machine(&body, machine_id);
    assert_eq!(entry["hostname"], "host-ac1");
    assert_eq!(entry["stale"], false);
    assert_rfc3339(entry, "last_seen");
    assert_rfc3339(entry, "last_message_at");
}

#[tokio::test]
async fn ac2_one_stale_machine_triggers_503_others_stay_accurate() {
    let fresh_id = Uuid::new_v4();
    let stale_id = Uuid::new_v4();
    let hub = spawn(&[fresh_id, stale_id]).await;

    let fresh_batch = sample_batch(
        fresh_id,
        "host-fresh",
        "sess-fresh",
        vec![msg(
            "sess-fresh",
            "k1",
            Some("u1"),
            "2026-01-01T00:00:00Z",
            "hi",
        )],
    );
    let stale_batch = sample_batch(
        stale_id,
        "host-stale",
        "sess-stale",
        vec![msg(
            "sess-stale",
            "k1",
            Some("u1"),
            "2026-01-01T00:00:00Z",
            "hi",
        )],
    );
    assert_eq!(ingest(&hub, fresh_id, &fresh_batch).await.status(), 200);
    assert_eq!(ingest(&hub, stale_id, &stale_batch).await.status(), 200);

    backdate_3h(&hub, stale_id).await;

    // Default threshold (7200s): our deliberately-backdated machine alone
    // guarantees a global "stale" verdict, regardless of any other machine's
    // state in the shared db.
    let resp = get_ingest_health(&hub, None).await;
    assert_eq!(resp.status(), 503);
    let body: Value = resp.json().await.expect("parse json body");
    assert_eq!(body["status"], "stale");

    assert_eq!(find_machine(&body, stale_id)["stale"], true);
    assert_eq!(find_machine(&body, fresh_id)["stale"], false);
}

#[tokio::test]
async fn ac3_stale_after_secs_threshold_is_honored_and_validated() {
    let machine_id = Uuid::new_v4();
    let hub = spawn(&[machine_id]).await;
    let batch = sample_batch(machine_id, "host-ac3", "sess-ac3", vec![]);
    assert_eq!(ingest(&hub, machine_id, &batch).await.status(), 200);
    backdate_3h(&hub, machine_id).await;

    // Default (7200s): a 3h-old last_seen exceeds it -> stale, and (as in
    // AC2) our own backdated machine alone guarantees the global 503.
    let resp = get_ingest_health(&hub, None).await;
    assert_eq!(resp.status(), 503);
    let body: Value = resp.json().await.expect("parse json body");
    assert_eq!(find_machine(&body, machine_id)["stale"], true);

    // Raised to 4h (14400s): this machine's 3h-old last_seen no longer
    // exceeds the raised threshold, so ITS `stale` flips to false. The
    // endpoint's overall HTTP status/`status` field at this larger threshold
    // depends on unrelated historical machines in the shared db and is
    // intentionally not asserted here — see module doc.
    let resp = get_ingest_health(&hub, Some("stale_after_secs=14400")).await;
    let body: Value = resp.json().await.expect("parse json body");
    assert_eq!(find_machine(&body, machine_id)["stale"], false);

    // Non-numeric or non-positive thresholds are rejected outright.
    for bad in ["abc", "0", "-100"] {
        let resp = get_ingest_health(&hub, Some(&format!("stale_after_secs={bad}"))).await;
        assert_eq!(
            resp.status(),
            400,
            "stale_after_secs={bad} must be rejected"
        );
    }
}

#[tokio::test]
async fn ac4_zero_messages_reports_null_last_message_at_and_not_stale() {
    let machine_id = Uuid::new_v4();
    let hub = spawn(&[machine_id]).await;
    // No messages ingested for this machine at all -> zero rows in `messages`.
    let batch = sample_batch(machine_id, "host-ac4", "sess-ac4", vec![]);
    assert_eq!(ingest(&hub, machine_id, &batch).await.status(), 200);

    // Huge threshold escapes shared-db pollution for the overall verdict —
    // see module doc.
    let resp = get_ingest_health(
        &hub,
        Some(&format!("stale_after_secs={HUGE_STALE_AFTER_SECS}")),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.expect("parse json body");
    assert_eq!(body["status"], "ok");

    let entry = find_machine(&body, machine_id);
    assert!(
        entry["last_message_at"].is_null(),
        "no messages ingested -> last_message_at must be null, got {:?}",
        entry["last_message_at"]
    );
    assert_eq!(entry["stale"], false);
}

#[tokio::test]
async fn ac5_no_auth_header_required() {
    let machine_id = Uuid::new_v4();
    let hub = spawn(&[machine_id]).await;
    let batch = sample_batch(machine_id, "host-ac5", "sess-ac5", vec![]);
    assert_eq!(ingest(&hub, machine_id, &batch).await.status(), 200);

    // get_ingest_health never sets an Authorization header. A correct
    // implementation answers with real content — 200 "ok" or 503 "stale"
    // depending on the shared db's staleness state — but never 401/403. This
    // also fails against the unmodified app: the route doesn't exist yet, so
    // it 404s instead of returning one of these two shapes.
    let resp = get_ingest_health(&hub, None).await;
    let status = resp.status().as_u16();
    assert!(
        status == 200 || status == 503,
        "expected 200 or 503 without an Authorization header, got {status}"
    );
}
