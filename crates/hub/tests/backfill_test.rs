//! Integration tests for the analytics backfill.
//!
//! The central claim under test is design D4: live ingest and the backfill are
//! one derivation, so a message derived at insert and the same message derived
//! retroactively must produce byte-identical rows. Each test therefore ingests
//! (deriving live), wipes the derived data to simulate a pre-existing row, runs
//! the backfill, and compares.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

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

/// A batch exercising every extraction path at once.
fn rich_batch(machine_id: Uuid) -> IngestBatch {
    batch(
        machine_id,
        "s1",
        vec![
            // Content-array invocations, incl. skill + subagent, plus the
            // redundant top-level restatement that must NOT double-count.
            msg(
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
                json!({ "messageId": "msg_01", "toolUse": { "name": "Read" } }),
            ),
            // Outcomes, one errored. Deliberately carries a large payload so the
            // backfill's slim projection is exercised against realistic bulk.
            msg(
                "s1",
                "k2",
                "user",
                json!([
                    { "type": "tool_result", "tool_use_id": "toolu_1",
                      "content": "x".repeat(200_000) },
                    { "type": "tool_result", "tool_use_id": "toolu_2", "is_error": true,
                      "content": "y".repeat(200_000) },
                ]),
                json!({}),
            ),
            // Top-level-only shape: no content-array invocation, so it counts.
            msg(
                "s1",
                "k3",
                "assistant",
                json!([{ "type": "text", "text": "x" }]),
                json!({ "toolUse": { "name": "Bash" },
                        "toolUseResult": { "is_error": true } }),
            ),
            // No tools at all.
            msg(
                "s1",
                "k4",
                "assistant",
                json!([{ "type": "text", "text": "nothing" }]),
                json!({ "messageId": "msg_04" }),
            ),
        ],
    )
}

type UseRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
);

/// (`message_key`, `tool_name`, `tool_use_id`, skill, subagent, `is_error`)
async fn use_rows(hub: &TestHub) -> Vec<UseRow> {
    sqlx::query_as::<_, UseRow>(
        r"
        SELECT m.message_key, u.tool_name, u.tool_use_id, u.skill_name, u.subagent_type, u.is_error
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

async fn result_rows(hub: &TestHub) -> Vec<(String, String, bool)> {
    sqlx::query_as::<_, (String, String, bool)>(
        r"
        SELECT m.message_key, r.tool_use_id, r.is_error
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

async fn id_rows(hub: &TestHub) -> Vec<(String, Option<String>)> {
    sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT message_key, message_id FROM messages WHERE machine_id = $1 ORDER BY message_key",
    )
    .bind(hub.machine_id)
    .fetch_all(&hub.pool)
    .await
    .unwrap()
}

/// Return the rows to the state a pre-backfill archive would be in.
async fn wipe_derived(hub: &TestHub) {
    sqlx::query(
        "DELETE FROM message_tool_uses u USING messages m
          WHERE m.id = u.message_ref AND m.machine_id = $1",
    )
    .bind(hub.machine_id)
    .execute(&hub.pool)
    .await
    .unwrap();
    sqlx::query(
        "DELETE FROM message_tool_results r USING messages m
          WHERE m.id = r.message_ref AND m.machine_id = $1",
    )
    .bind(hub.machine_id)
    .execute(&hub.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE messages SET message_id = NULL WHERE machine_id = $1")
        .bind(hub.machine_id)
        .execute(&hub.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn backfill_reproduces_exactly_what_live_ingest_derived() {
    // D4's central claim: one derivation, two callers. If these ever diverge,
    // the archive's history means something different from its present.
    let hub = spawn().await;
    assert_eq!(post_ingest(&hub, &rich_batch(hub.machine_id)).await, 200);

    let live_uses = use_rows(&hub).await;
    let live_results = result_rows(&hub).await;
    let live_ids = id_rows(&hub).await;
    assert!(!live_uses.is_empty(), "live ingest derived nothing");

    wipe_derived(&hub).await;
    assert!(use_rows(&hub).await.is_empty());
    assert!(id_rows(&hub).await.iter().all(|(_, id)| id.is_none()));

    hub::backfill::run(&hub.pool, 500).await.unwrap();

    assert_eq!(use_rows(&hub).await, live_uses, "invocations diverged");
    assert_eq!(result_rows(&hub).await, live_results, "outcomes diverged");
    assert_eq!(id_rows(&hub).await, live_ids, "message_id diverged");
}

#[tokio::test]
async fn message_id_sql_matches_rust_extractor() {
    // The message_id backfill is SQL while live ingest is Rust — the one place
    // the "single implementation" rule is knowingly bent, so it is pinned here.
    let hub = spawn().await;
    let b = batch(
        hub.machine_id,
        "s1",
        vec![
            msg(
                "s1",
                "k1",
                "assistant",
                json!([]),
                json!({ "messageId": "msg_01ABC" }),
            ),
            msg("s1", "k2", "user", json!([]), json!({})),
            // Empty string must be treated as absent by BOTH sides.
            msg(
                "s1",
                "k3",
                "assistant",
                json!([]),
                json!({ "messageId": "" }),
            ),
            // The nested provider shape is not what `raw` holds; both sides
            // must decline to read it.
            msg(
                "s1",
                "k4",
                "assistant",
                json!([]),
                json!({ "message": { "id": "msg_NESTED" } }),
            ),
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let by_rust = id_rows(&hub).await;
    wipe_derived(&hub).await;
    hub::backfill::run(&hub.pool, 500).await.unwrap();
    let by_sql = id_rows(&hub).await;

    assert_eq!(by_sql, by_rust);
    assert_eq!(
        by_sql,
        vec![
            ("k1".to_owned(), Some("msg_01ABC".to_owned())),
            ("k2".to_owned(), None),
            ("k3".to_owned(), None),
            ("k4".to_owned(), None),
        ]
    );
}

#[tokio::test]
async fn projection_feeds_the_extractor_identically() {
    // The backfill hands the extractor a slimmed `content` (tool_result bodies
    // can be tens of MiB). If the projection ever omits a key the extractor
    // reads, the backfill silently derives less than live ingest — which the
    // wipe-and-compare below would catch as a diff.
    let hub = spawn().await;
    assert_eq!(post_ingest(&hub, &rich_batch(hub.machine_id)).await, 200);

    let live_uses = use_rows(&hub).await;
    // Every field the projection carries must survive the round trip.
    assert!(live_uses.iter().any(|r| r.3.is_some()), "no skill derived");
    assert!(
        live_uses.iter().any(|r| r.4.is_some()),
        "no subagent derived"
    );
    assert!(live_uses.iter().any(|r| r.2.is_some()), "no tool_use_id");
    assert!(live_uses.iter().any(|r| r.5), "no errored invocation");

    wipe_derived(&hub).await;
    hub::backfill::run(&hub.pool, 500).await.unwrap();

    assert_eq!(use_rows(&hub).await, live_uses);
}

#[tokio::test]
async fn backfill_is_idempotent_and_resumable() {
    let hub = spawn().await;
    assert_eq!(post_ingest(&hub, &rich_batch(hub.machine_id)).await, 200);
    let expected_uses = use_rows(&hub).await;
    let expected_results = result_rows(&hub).await;

    wipe_derived(&hub).await;

    // A batch size of 1 forces many cursor advances — the resumption path.
    hub::backfill::run(&hub.pool, 1).await.unwrap();
    assert_eq!(use_rows(&hub).await, expected_uses);
    assert_eq!(result_rows(&hub).await, expected_results);

    // Running again over already-derived rows changes nothing.
    hub::backfill::run(&hub.pool, 500).await.unwrap();
    assert_eq!(use_rows(&hub).await, expected_uses);
    assert_eq!(result_rows(&hub).await, expected_results);
}

#[tokio::test]
async fn top_level_restatement_is_not_double_counted_by_the_backfill() {
    // D12 must hold on the backfill path too, not just at ingest — otherwise
    // history and present would report different tool counts.
    let hub = spawn().await;
    assert_eq!(post_ingest(&hub, &rich_batch(hub.machine_id)).await, 200);
    wipe_derived(&hub).await;
    hub::backfill::run(&hub.pool, 500).await.unwrap();

    let rows = use_rows(&hub).await;
    // k1 restates "Read" at top level alongside three array invocations.
    let k1: Vec<_> = rows.iter().filter(|r| r.0 == "k1").collect();
    assert_eq!(k1.len(), 3, "top-level restatement was counted again");
    assert_eq!(
        k1.iter().filter(|r| r.1 == "Read").count(),
        1,
        "Read counted twice"
    );
    // k3 has only the top-level shape, so it must still be counted.
    let k3: Vec<_> = rows.iter().filter(|r| r.0 == "k3").collect();
    assert_eq!(k3.len(), 1);
    assert_eq!(k3[0].1, "Bash");
    assert!(k3[0].5, "same-record error lost");
}
