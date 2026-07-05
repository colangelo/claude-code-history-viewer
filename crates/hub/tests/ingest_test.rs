//! Integration tests for the hub ingest endpoint.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).
//! Each test uses a fresh random `machine_id` so data is isolated within one
//! shared database. Migrations are applied via the crate's `MIGRATOR` (sqlx
//! takes an advisory lock, so concurrent test runs are safe).

use archive_protocol::{
    IngestBatch, IngestMessage, IngestProject, IngestResponse, IngestSession, MachineInfo,
};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use uuid::Uuid;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests")
}

/// A running test hub: base URL + the token/machine identity it trusts + a pool
/// for direct assertions.
struct TestHub {
    base: String,
    token: String,
    machine_id: Uuid,
    pool: PgPool,
}

async fn spawn() -> TestHub {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");

    let machine_id = Uuid::new_v4();
    let token = format!("tok-{machine_id}");
    let mut tokens = HashMap::new();
    tokens.insert(token.clone(), machine_id);

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
        token,
        machine_id,
        pool,
    }
}

async fn post_ingest(hub: &TestHub, token: Option<&str>, batch: &IngestBatch) -> reqwest::Response {
    let client = reqwest::Client::new();
    let mut req = client.post(format!("{}/v1/ingest", hub.base)).json(batch);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.expect("send ingest request")
}

/// One project + one session + `n` messages for the given machine.
fn sample_batch(machine_id: Uuid, session: &str, messages: Vec<IngestMessage>) -> IngestBatch {
    IngestBatch {
        machine: MachineInfo {
            machine_id,
            hostname: "testbox".into(),
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

async fn message_count(hub: &TestHub) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM messages WHERE machine_id = $1")
        .bind(hub.machine_id)
        .fetch_one(&hub.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn valid_ingest_persists_and_counts() {
    let hub = spawn().await;
    let batch = sample_batch(
        hub.machine_id,
        "sess-1",
        vec![
            msg(
                "sess-1",
                "k1",
                Some("u1"),
                "2026-01-01T00:00:00Z",
                "hello world",
            ),
            msg(
                "sess-1",
                "k2",
                Some("u2"),
                "2026-01-01T00:01:00Z",
                "second message",
            ),
        ],
    );
    let resp = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(resp.status(), 200);
    let body: IngestResponse = resp.json().await.unwrap();
    assert_eq!(body.projects_inserted, 1);
    assert_eq!(body.sessions_inserted, 1);
    assert_eq!(body.messages_inserted, 2);
    assert_eq!(message_count(&hub).await, 2);
}

#[tokio::test]
async fn missing_token_is_401() {
    let hub = spawn().await;
    let batch = sample_batch(hub.machine_id, "sess-x", vec![]);
    let resp = post_ingest(&hub, None, &batch).await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn invalid_token_is_401() {
    let hub = spawn().await;
    let batch = sample_batch(hub.machine_id, "sess-x", vec![]);
    let resp = post_ingest(&hub, Some("not-a-real-token"), &batch).await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn message_for_unknown_session_is_400_with_no_partial_write() {
    let hub = spawn().await;
    // A valid session S1, plus a message referencing an unknown session S2.
    let mut batch = sample_batch(
        hub.machine_id,
        "sess-known",
        vec![msg(
            "sess-known",
            "k1",
            Some("u1"),
            "2026-01-01T00:00:00Z",
            "ok",
        )],
    );
    batch.messages.push(msg(
        "sess-UNKNOWN",
        "k2",
        Some("u2"),
        "2026-01-01T00:02:00Z",
        "orphan",
    ));
    let resp = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(resp.status(), 400);
    // Whole batch rolled back: not even the valid session/message persisted.
    assert_eq!(message_count(&hub).await, 0);
    let machines: i64 = sqlx::query_scalar("SELECT count(*) FROM machines WHERE machine_id = $1")
        .bind(hub.machine_id)
        .fetch_one(&hub.pool)
        .await
        .unwrap();
    assert_eq!(machines, 0, "machine upsert must roll back with the batch");
}

#[tokio::test]
async fn double_post_is_idempotent() {
    let hub = spawn().await;
    let batch = sample_batch(
        hub.machine_id,
        "sess-dup",
        vec![
            msg("sess-dup", "k1", Some("u1"), "2026-01-01T00:00:00Z", "one"),
            msg("sess-dup", "k2", Some("u2"), "2026-01-01T00:01:00Z", "two"),
        ],
    );
    let first: IngestResponse = post_ingest(&hub, Some(&hub.token), &batch)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(first.messages_inserted, 2);

    let second: IngestResponse = post_ingest(&hub, Some(&hub.token), &batch)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(second.messages_inserted, 0);
    assert_eq!(second.messages_skipped, 2);
    assert_eq!(
        message_count(&hub).await,
        2,
        "no duplicate rows after re-post"
    );
}

#[tokio::test]
async fn raw_jsonb_round_trips() {
    let hub = spawn().await;
    let raw = json!({ "nested": { "a": 1, "b": [true, null, "x"] }, "orig": "verbatim" });
    let mut m = msg("sess-raw", "k1", Some("u1"), "2026-01-01T00:00:00Z", "body");
    m.raw = raw.clone();
    let batch = sample_batch(hub.machine_id, "sess-raw", vec![m]);
    let resp = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(resp.status(), 200);

    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT raw FROM messages WHERE machine_id = $1 AND message_key = 'k1'")
            .bind(hub.machine_id)
            .fetch_one(&hub.pool)
            .await
            .unwrap();
    assert_eq!(stored, raw, "raw JSONB must round-trip verbatim");
}

#[tokio::test]
async fn uuidless_provider_dedups_by_content_key() {
    let hub = spawn().await;
    // No uuid; message_key is a content-derived key. Same key twice → 1 row.
    let batch = sample_batch(
        hub.machine_id,
        "sess-nouuid",
        vec![
            msg(
                "sess-nouuid",
                "content-hash-abc",
                None,
                "2026-01-01T00:00:00Z",
                "x",
            ),
            msg(
                "sess-nouuid",
                "content-hash-abc",
                None,
                "2026-01-01T00:00:00Z",
                "x",
            ),
        ],
    );
    let body: IngestResponse = post_ingest(&hub, Some(&hub.token), &batch)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(body.messages_inserted, 1);
    assert_eq!(body.messages_skipped, 1);
    assert_eq!(message_count(&hub).await, 1);
}

#[tokio::test]
async fn aggregates_update_on_reingest() {
    let hub = spawn().await;
    // First batch: 1 message.
    let b1 = sample_batch(
        hub.machine_id,
        "sess-agg",
        vec![msg(
            "sess-agg",
            "k1",
            Some("u1"),
            "2026-01-01T00:00:00Z",
            "first",
        )],
    );
    post_ingest(&hub, Some(&hub.token), &b1).await;

    // Second batch: the same session gains a later message.
    let b2 = sample_batch(
        hub.machine_id,
        "sess-agg",
        vec![msg(
            "sess-agg",
            "k2",
            Some("u2"),
            "2026-01-02T00:00:00Z",
            "second",
        )],
    );
    post_ingest(&hub, Some(&hub.token), &b2).await;

    let (count, last): (i32, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT message_count, last_message_time FROM sessions WHERE machine_id = $1 AND session_id = 'sess-agg'",
    )
    .bind(hub.machine_id)
    .fetch_one(&hub.pool)
    .await
    .unwrap();
    assert_eq!(count, 2, "cumulative message_count reflects both batches");
    assert_eq!(
        last.unwrap().to_rfc3339(),
        "2026-01-02T00:00:00+00:00",
        "last_message_time advanced to the newer message"
    );
}

#[tokio::test]
async fn ingest_sanitizes_nul_characters() {
    let hub = spawn().await;
    // Postgres rejects U+0000 in jsonb and TEXT; real transcripts contain it
    // (raw terminal output). The hub must sanitize instead of 500-ing.
    let text = "terminal output\0with a NUL\0byte";
    let batch = sample_batch(
        hub.machine_id,
        "sess-nul",
        vec![msg(
            "sess-nul",
            "k-nul",
            Some("u-nul"),
            "2026-07-03T12:00:00Z",
            text,
        )],
    );

    let resp = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(resp.status(), 200);
    let body: IngestResponse = resp.json().await.expect("parse response");
    assert_eq!(body.messages_inserted, 1);

    let (search_text, raw): (String, serde_json::Value) = sqlx::query_as(
        "SELECT search_text, raw FROM messages WHERE machine_id = $1 AND message_key = 'k-nul'",
    )
    .bind(hub.machine_id)
    .fetch_one(&hub.pool)
    .await
    .expect("stored row");

    assert!(!search_text.contains('\0'), "NUL must not reach TEXT");
    assert!(
        search_text.contains('\u{FFFD}'),
        "NUL replaced, not dropped"
    );
    let raw_str = raw.to_string();
    assert!(!raw_str.contains('\0'), "NUL must not reach jsonb");
    assert!(raw_str.contains('\u{FFFD}'), "raw preserves a marker");
}

/// gitea #7: a message whose `search_text` exceeds Postgres's 1 MiB tsvector
/// limit must not fail the batch — the hub clamps `search_text` at ingest
/// (raw/content keep full fidelity). Hit in practice by Time Machine
/// backfills of old sessions.
#[tokio::test]
async fn oversized_search_text_is_clamped_not_rejected() {
    let hub = spawn().await;
    // Distinct words so the tsvector genuinely grows with input size.
    let mut huge = String::new();
    for i in 0..200_000 {
        use std::fmt::Write;
        write!(huge, "w{i} ").unwrap();
    }
    assert!(
        huge.len() > 1_048_575,
        "test input must exceed the pg limit"
    );
    let batch = sample_batch(
        hub.machine_id,
        "sess-huge",
        vec![msg(
            "sess-huge",
            "k-huge",
            Some("u-huge"),
            "2026-03-27T17:17:03Z",
            &huge,
        )],
    );
    let res = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(res.status(), 200, "oversized message must ingest");

    let stored: String = sqlx::query_scalar(
        "SELECT search_text FROM messages WHERE machine_id = $1 AND message_key = 'k-huge'",
    )
    .bind(hub.machine_id)
    .fetch_one(&hub.pool)
    .await
    .expect("row must exist");
    assert!(stored.len() <= 512 * 1024, "search_text must be clamped");
    assert!(stored.starts_with("w0 w1 "), "head of text preserved");
}
