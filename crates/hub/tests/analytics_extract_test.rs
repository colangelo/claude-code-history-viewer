//! Integration tests for the analytics fields derived at ingest: the provider
//! `message_id` column, `message_tool_uses`, and `message_tool_results`.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).
//! Each test uses a fresh random `machine_id` so data is isolated within one
//! shared database.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::collections::HashMap;
use tokio::net::TcpListener;
use uuid::Uuid;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests")
}

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

    let state = hub::AppState::new(pool.clone(), tokens, Vec::new());
    let app = hub::router(state, None);
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

async fn post_ingest(hub: &TestHub, batch: &IngestBatch) -> reqwest::StatusCode {
    reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.token)
        .json(batch)
        .send()
        .await
        .expect("post ingest")
        .status()
}

fn batch(machine_id: Uuid, session: &str, messages: Vec<IngestMessage>) -> IngestBatch {
    IngestBatch {
        machine: MachineInfo {
            machine_id,
            hostname: "test-host".into(),
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
            ..Default::default()
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
            has_tool_use: Some(true),
            has_errors: Some(false),
            storage_type: Some("jsonl".into()),
        }],
        messages,
    }
}

/// A message with explicit type, content and raw — the three inputs extraction
/// actually reads.
fn msg(session: &str, key: &str, message_type: &str, content: Value, raw: Value) -> IngestMessage {
    IngestMessage {
        provider: "claude".into(),
        session_id: session.into(),
        message_key: key.into(),
        uuid: Some(format!("uuid-{key}")),
        parent_uuid: None,
        seq: 0,
        timestamp: Some("2026-07-25T10:00:00Z".into()),
        message_type: Some(message_type.into()),
        role: Some(message_type.into()),
        model: Some("claude-opus-5".into()),
        stop_reason: None,
        input_tokens: Some(10),
        output_tokens: Some(20),
        cache_creation_tokens: None,
        cache_read_tokens: None,
        cost_usd: None,
        duration_ms: None,
        is_sidechain: false,
        content: Some(content),
        raw,
        search_text: Some("text".into()),
    }
}

async fn message_ids(hub: &TestHub) -> Vec<Option<String>> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT message_id FROM messages WHERE machine_id = $1 ORDER BY message_key",
    )
    .bind(hub.machine_id)
    .fetch_all(&hub.pool)
    .await
    .unwrap()
}

/// (`tool_name`, `tool_use_id`, `skill_name`, `subagent_type`, `is_error`) for
/// the hub's machine, ordered stably.
type ToolRow = (String, Option<String>, Option<String>, Option<String>, bool);

async fn tool_uses(hub: &TestHub) -> Vec<ToolRow> {
    sqlx::query_as::<_, ToolRow>(
        r"
        SELECT u.tool_name, u.tool_use_id, u.skill_name, u.subagent_type, u.is_error
        FROM message_tool_uses u
        JOIN messages m ON m.id = u.message_ref
        WHERE m.machine_id = $1
        ORDER BY m.message_key, u.seq
        ",
    )
    .bind(hub.machine_id)
    .fetch_all(&hub.pool)
    .await
    .unwrap()
}

async fn tool_results(hub: &TestHub) -> Vec<(String, bool)> {
    sqlx::query_as::<_, (String, bool)>(
        r"
        SELECT r.tool_use_id, r.is_error
        FROM message_tool_results r
        JOIN messages m ON m.id = r.message_ref
        WHERE m.machine_id = $1
        ORDER BY m.message_key, r.seq
        ",
    )
    .bind(hub.machine_id)
    .fetch_all(&hub.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn provider_message_id_lands_in_its_column() {
    let hub = spawn().await;
    let b = batch(
        hub.machine_id,
        "s1",
        vec![
            msg(
                "s1",
                "k1",
                "assistant",
                json!([{ "type": "text", "text": "hi" }]),
                json!({ "messageId": "msg_01ABC" }),
            ),
            // No id at all — stays NULL, ingest still succeeds.
            msg(
                "s1",
                "k2",
                "user",
                json!([{ "type": "text", "text": "yo" }]),
                json!({}),
            ),
            // The pre-normalization nested shape must NOT be read: `raw` is the
            // flat normalized record, so this yields NULL, not "msg_NESTED".
            msg(
                "s1",
                "k3",
                "assistant",
                json!([{ "type": "text", "text": "x" }]),
                json!({ "message": { "id": "msg_NESTED" } }),
            ),
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);
    assert_eq!(
        message_ids(&hub).await,
        vec![Some("msg_01ABC".to_owned()), None, None]
    );
}

#[tokio::test]
async fn tool_invocations_skills_and_subagents_are_extracted() {
    let hub = spawn().await;
    let b = batch(
        hub.machine_id,
        "s1",
        vec![msg(
            "s1",
            "k1",
            "assistant",
            json!([
                { "type": "tool_use", "id": "toolu_1", "name": "Read", "input": {} },
                { "type": "tool_use", "id": "toolu_2", "name": "Skill",
                  "input": { "skill": "cchv-find" } },
                { "type": "tool_use", "id": "toolu_3", "name": "Agent",
                  "input": { "subagent_type": "Explore" } },
            ]),
            json!({ "messageId": "msg_01" }),
        )],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let rows = tool_uses(&hub).await;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].0, "Read");
    assert_eq!(rows[1].0, "Skill");
    assert_eq!(rows[1].2, Some("cchv-find".to_owned()));
    assert_eq!(rows[2].0, "Agent");
    assert_eq!(rows[2].3, Some("Explore".to_owned()));
    // None self-flagged: outcomes arrive separately.
    assert!(rows.iter().all(|r| !r.4));
}

#[tokio::test]
async fn tool_outcomes_are_extracted_from_results() {
    let hub = spawn().await;
    let b = batch(
        hub.machine_id,
        "s1",
        vec![
            msg(
                "s1",
                "k1",
                "assistant",
                json!([{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {} }]),
                json!({}),
            ),
            msg(
                "s1",
                "k2",
                "user",
                json!([{ "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true }]),
                json!({}),
            ),
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    assert_eq!(tool_uses(&hub).await.len(), 1);
    assert_eq!(tool_results(&hub).await, vec![("toolu_1".to_owned(), true)]);
}

#[tokio::test]
async fn result_ingested_before_its_invocation_still_resolves() {
    // Order independence: the outcome may land in an earlier batch than the
    // invocation it reports on. Nothing reconciles them at write time — the
    // join at query time is what pairs them.
    let hub = spawn().await;

    let first = batch(
        hub.machine_id,
        "s1",
        vec![msg(
            "s1",
            "k2",
            "user",
            json!([{ "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true }]),
            json!({}),
        )],
    );
    assert_eq!(post_ingest(&hub, &first).await, 200);
    assert!(tool_uses(&hub).await.is_empty());
    assert_eq!(tool_results(&hub).await.len(), 1);

    let second = batch(
        hub.machine_id,
        "s1",
        vec![msg(
            "s1",
            "k1",
            "assistant",
            json!([{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {} }]),
            json!({}),
        )],
    );
    assert_eq!(post_ingest(&hub, &second).await, 200);

    // The pair is now joinable, and the outcome wins over the invocation's own
    // (never-set) flag — this is the shape the success-rate rollup uses.
    let resolved: Vec<(String, bool)> = sqlx::query_as(
        r"
        SELECT u.tool_name, COALESCE(r.is_error, u.is_error, false)
        FROM message_tool_uses u
        JOIN messages m ON m.id = u.message_ref
        LEFT JOIN message_tool_results r
               ON r.session_id = u.session_id AND r.tool_use_id = u.tool_use_id
        WHERE m.machine_id = $1
        ",
    )
    .bind(hub.machine_id)
    .fetch_all(&hub.pool)
    .await
    .unwrap();
    assert_eq!(resolved, vec![("Bash".to_owned(), true)]);
}

#[tokio::test]
async fn top_level_tool_use_resolves_on_its_own_record() {
    let hub = spawn().await;
    let b = batch(
        hub.machine_id,
        "s1",
        vec![msg(
            "s1",
            "k1",
            "user",
            json!([{ "type": "text", "text": "x" }]),
            json!({ "toolUse": { "name": "Bash" }, "toolUseResult": { "is_error": true } }),
        )],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let rows = tool_uses(&hub).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "Bash");
    assert_eq!(rows[0].1, None, "top-level shape carries no tool_use_id");
    assert!(rows[0].4, "same-record result is flagged on the invocation");
}

#[tokio::test]
async fn reingest_does_not_duplicate_derived_rows() {
    let hub = spawn().await;
    let b = batch(
        hub.machine_id,
        "s1",
        vec![
            msg(
                "s1",
                "k1",
                "assistant",
                json!([
                    { "type": "tool_use", "id": "toolu_1", "name": "Read", "input": {} },
                    { "type": "tool_use", "id": "toolu_2", "name": "Bash", "input": {} },
                ]),
                json!({ "messageId": "msg_01" }),
            ),
            msg(
                "s1",
                "k2",
                "user",
                json!([{ "type": "tool_result", "tool_use_id": "toolu_1" }]),
                json!({}),
            ),
        ],
    );

    assert_eq!(post_ingest(&hub, &b).await, 200);
    let uses_once = tool_uses(&hub).await;
    let results_once = tool_results(&hub).await;
    assert_eq!(uses_once.len(), 2);
    assert_eq!(results_once.len(), 1);

    // Same batch again: the message upsert conflicts, so nothing is re-derived.
    assert_eq!(post_ingest(&hub, &b).await, 200);
    assert_eq!(tool_uses(&hub).await, uses_once);
    assert_eq!(tool_results(&hub).await, results_once);
}

#[tokio::test]
async fn messages_without_tool_use_derive_nothing() {
    let hub = spawn().await;
    let b = batch(
        hub.machine_id,
        "s1",
        vec![msg(
            "s1",
            "k1",
            "assistant",
            json!([{ "type": "text", "text": "no tools here" }]),
            json!({ "messageId": "msg_01" }),
        )],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);
    assert!(tool_uses(&hub).await.is_empty());
    assert!(tool_results(&hub).await.is_empty());
}
