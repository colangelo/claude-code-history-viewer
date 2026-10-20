//! Integration tests for ingest-only machine tokens
//! (`openspec/changes/ingest-only-tokens`). An ingest-only token may write its
//! own sessions through `/v1/ingest` and nothing else: every read route and
//! every other machine-token write (journal entries, distiller ticks) answers
//! 401, so a fenced guest can archive without reading the archive.
//! Requires `TEST_DATABASE_URL`/`DATABASE_URL`.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::collections::{HashMap, HashSet};
use tokio::net::TcpListener;
use uuid::Uuid;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

struct TestHub {
    base: String,
    full: (String, Uuid),
    ingest_only: (String, Uuid),
}

async fn spawn() -> TestHub {
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&test_db_url())
        .await
        .expect("connect");
    hub::MIGRATOR.run(&pool).await.expect("migrate");

    let full = (format!("tok-full-{}", Uuid::new_v4()), Uuid::new_v4());
    let ingest_only = (format!("tok-ingest-{}", Uuid::new_v4()), Uuid::new_v4());
    let mut tokens = HashMap::new();
    tokens.insert(full.0.clone(), full.1);
    tokens.insert(ingest_only.0.clone(), ingest_only.1);

    let state = hub::AppState::new(pool, tokens, Vec::new())
        .with_ingest_only_tokens(HashSet::from([ingest_only.0.clone()]));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, hub::router(state, None))
            .await
            .unwrap();
    });
    TestHub {
        base: format!("http://{addr}"),
        full,
        ingest_only,
    }
}

fn batch(machine_id: Uuid, hostname: &str) -> IngestBatch {
    let session = format!("scope-{}", Uuid::new_v4());
    IngestBatch {
        machine: MachineInfo {
            machine_id,
            hostname: hostname.into(),
            os: Some("linux".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: "/tmp/scope-proj".into(),
            name: Some("scope-proj".into()),
            storage_type: Some("jsonl".into()),
            session_count: Some(1),
            message_count: Some(1),
            last_modified: None,
            ..Default::default()
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: session.clone(),
            project_path: Some("/tmp/scope-proj".into()),
            file_path: Some(format!("/tmp/scope-proj/{session}.jsonl")),
            entrypoint: None,
            summary: None,
            message_count: Some(1),
            first_message_time: None,
            last_message_time: None,
            last_modified: None,
            has_tool_use: Some(false),
            has_errors: Some(false),
            storage_type: Some("jsonl".into()),
        }],
        messages: vec![IngestMessage {
            provider: "claude".into(),
            session_id: session,
            message_key: format!("k-{}", Uuid::new_v4()),
            uuid: None,
            parent_uuid: None,
            seq: 0,
            timestamp: Some("2026-10-21T00:00:00Z".into()),
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
            content: Some(json!([{ "type": "text", "text": "hello" }])),
            raw: json!({ "text": "hello" }),
            search_text: Some("hello".into()),
        }],
    }
}

/// Every read route family the archive exposes, plus the two non-ingest
/// machine-token writes. Each must refuse an ingest-only token.
const READ_ROUTES: [&str; 6] = [
    "/v1/projects",
    "/v1/sessions",
    "/v1/search?q=hello",
    "/v1/findings/summary",
    "/v1/journal/pending",
    "/v1/stats/global",
];

#[tokio::test]
async fn ingest_only_token_can_ingest() {
    let hub = spawn().await;
    let res = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.ingest_only.0)
        .json(&batch(hub.ingest_only.1, "agent1"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        200,
        "{}",
        res.text().await.unwrap_or_default()
    );
}

#[tokio::test]
async fn ingest_only_token_is_refused_on_every_read_route() {
    let hub = spawn().await;
    let client = reqwest::Client::new();
    for route in READ_ROUTES {
        let res = client
            .get(format!("{}{route}", hub.base))
            .bearer_auth(&hub.ingest_only.0)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 401, "ingest-only token on GET {route}");
    }
}

#[tokio::test]
async fn ingest_only_token_cannot_write_the_journal() {
    let hub = spawn().await;
    let client = reqwest::Client::new();
    for route in ["/v1/journal/entries", "/v1/journal/ticks"] {
        let res = client
            .post(format!("{}{route}", hub.base))
            .bearer_auth(&hub.ingest_only.0)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 401, "ingest-only token on POST {route}");
    }
}

#[tokio::test]
async fn full_token_still_reads_and_ingests() {
    let hub = spawn().await;
    let client = reqwest::Client::new();
    let res = client
        .get(format!("{}/v1/projects", hub.base))
        .bearer_auth(&hub.full.0)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let res = client
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.full.0)
        .json(&batch(hub.full.1, "fullbox"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn unknown_token_is_still_401_on_ingest() {
    let hub = spawn().await;
    let res = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth("tok-nobody")
        .json(&batch(Uuid::new_v4(), "nobody"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}
