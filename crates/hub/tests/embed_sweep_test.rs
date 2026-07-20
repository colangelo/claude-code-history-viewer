//! Integration tests for the journal embedding sweep (stub embedder):
//! bootstrap, hash-driven re-embedding, skip exclusion, model change, and
//! self-healing after row deletion.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).
//! Each test uses a fresh random `machine_id`; project paths embed it so data
//! is isolated within the shared test database.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use tokio::net::TcpListener;
use uuid::Uuid;

use hub::embed::StubEmbedder;
use hub::embed_sweep::{sweep, SweepStats};

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

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// Ingest one session with one message at `path` (project fingerprint-less —
/// identity is irrelevant here), first message at noon UTC on `date`.
async fn seed_session(hub: &TestHub, path: &str, session: &str, date: &str) {
    let ts = format!("{date}T12:00:00Z");
    let batch = IngestBatch {
        machine: MachineInfo {
            machine_id: hub.machine_id,
            hostname: format!("host-{}", hub.machine_id),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: path.into(),
            name: Some("proj".into()),
            ..Default::default()
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: session.into(),
            project_path: Some(path.into()),
            file_path: Some(format!("{path}/{session}.jsonl")),
            entrypoint: None,
            summary: Some("s".into()),
            message_count: Some(1),
            first_message_time: Some(ts.clone()),
            last_message_time: Some(ts.clone()),
            last_modified: Some(ts.clone()),
            has_tool_use: Some(false),
            has_errors: Some(false),
            storage_type: Some("jsonl".into()),
        }],
        messages: vec![IngestMessage {
            provider: "claude".into(),
            session_id: session.into(),
            message_key: format!("{session}-m1"),
            uuid: Some(format!("{session}-u1")),
            parent_uuid: None,
            seq: 0,
            timestamp: Some(ts.clone()),
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
            content: Some(json!([{ "type": "text", "text": "seed" }])),
            raw: json!({ "text": "seed" }),
            search_text: Some("seed".into()),
        }],
    };
    let resp = client()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.token)
        .json(&batch)
        .send()
        .await
        .expect("ingest");
    assert_eq!(resp.status(), 200, "ingest failed");
}

/// Session surrogate ids for `path` (needed for journal-entry provenance).
async fn session_ids(hub: &TestHub, path: &str) -> Vec<i64> {
    let resp = client()
        .get(format!("{}/v1/sessions?project={}", hub.base, path))
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let sessions: Value = resp.json().await.unwrap();
    sessions
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_i64().unwrap())
        .collect()
}

/// POST a journal row for `path`/`date`. `status` is `entry` or `skip`.
async fn post_entry(hub: &TestHub, path: &str, date: &str, status: &str, summary: &str) {
    let ids = session_ids(hub, path).await;
    assert!(!ids.is_empty());
    let mut payload = json!({
        "entry_date": date,
        "project_path": path,
        "status": status,
        "session_ids": ids,
    });
    if status == "entry" {
        payload["headline"] = json!("Test headline");
        payload["summary"] = json!(summary);
        payload["topics"] = json!(["alpha", "beta", "gamma"]);
        payload["open_questions"] = json!([]);
        payload["model"] = json!("test-model");
    }
    let resp = client()
        .post(format!("{}/v1/journal/entries", hub.base))
        .bearer_auth(&hub.token)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "journal entry create");
}

/// `(journal_entry_id, model, content_hash)` rows for this test's project paths.
async fn embedding_rows(pool: &PgPool, machine_id: Uuid) -> Vec<(i64, String, String)> {
    sqlx::query(
        r"
        SELECT e.journal_entry_id, e.model, e.content_hash
        FROM journal_embeddings e
        JOIN journal_entries je ON je.id = e.journal_entry_id
        WHERE je.project_path LIKE '/tmp/emb-' || $1 || '%'
        ORDER BY e.journal_entry_id, e.model
        ",
    )
    .bind(machine_id.to_string())
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
    .collect()
}

fn stub() -> StubEmbedder {
    StubEmbedder::new("stub-model", 8)
}

#[tokio::test]
async fn bootstrap_embeds_existing_entries_once() {
    let hub = spawn().await;
    let m = hub.machine_id;
    let p1 = format!("/tmp/emb-{m}/one");
    let p2 = format!("/tmp/emb-{m}/two");
    seed_session(&hub, &p1, &format!("emb1-{m}"), "2026-07-01").await;
    seed_session(&hub, &p2, &format!("emb2-{m}"), "2026-07-02").await;
    post_entry(&hub, &p1, "2026-07-01", "entry", "First summary.").await;
    post_entry(&hub, &p2, "2026-07-02", "entry", "Second summary.").await;

    // Bootstrap: both entries embed. (The shared test DB may hold other
    // rows; scope assertions to this test's paths.)
    sweep(&hub.pool, &stub()).await.unwrap();
    assert_eq!(embedding_rows(&hub.pool, m).await.len(), 2);

    // A second pass over OUR rows is a no-op: hashes unchanged.
    let before = embedding_rows(&hub.pool, m).await;
    sweep(&hub.pool, &stub()).await.unwrap();
    assert_eq!(embedding_rows(&hub.pool, m).await, before);
}

#[tokio::test]
async fn regenerated_entry_re_embeds_on_hash_change() {
    let hub = spawn().await;
    let m = hub.machine_id;
    let p = format!("/tmp/emb-{m}/regen");
    seed_session(&hub, &p, &format!("regen-{m}"), "2026-07-03").await;
    post_entry(&hub, &p, "2026-07-03", "entry", "Original summary.").await;
    sweep(&hub.pool, &stub()).await.unwrap();
    let first = embedding_rows(&hub.pool, m).await;
    assert_eq!(first.len(), 1);

    // Same group key, new content → upsert in place → hash goes stale.
    post_entry(&hub, &p, "2026-07-03", "entry", "Regenerated summary.").await;
    sweep(&hub.pool, &stub()).await.unwrap();
    let second = embedding_rows(&hub.pool, m).await;
    assert_eq!(second.len(), 1);
    assert_eq!(first[0].0, second[0].0, "same entry row");
    assert_ne!(first[0].2, second[0].2, "content hash replaced");
}

#[tokio::test]
async fn skip_rows_are_never_embedded() {
    let hub = spawn().await;
    let m = hub.machine_id;
    let p = format!("/tmp/emb-{m}/skip");
    seed_session(&hub, &p, &format!("skip-{m}"), "2026-07-04").await;
    post_entry(&hub, &p, "2026-07-04", "skip", "").await;
    let stats = sweep(&hub.pool, &stub()).await.unwrap();
    assert_eq!(embedding_rows(&hub.pool, m).await.len(), 0);
    assert_eq!(stats.failed, 0);
}

#[tokio::test]
async fn model_change_embeds_into_the_new_space() {
    let hub = spawn().await;
    let m = hub.machine_id;
    let p = format!("/tmp/emb-{m}/model");
    seed_session(&hub, &p, &format!("model-{m}"), "2026-07-05").await;
    post_entry(&hub, &p, "2026-07-05", "entry", "Model summary.").await;
    sweep(&hub.pool, &stub()).await.unwrap();

    // New active model: the entry is missing in that space → embeds again.
    let v2 = StubEmbedder::new("stub-model-v2", 8);
    sweep(&hub.pool, &v2).await.unwrap();
    let rows = embedding_rows(&hub.pool, m).await;
    let models: Vec<&str> = rows.iter().map(|r| r.1.as_str()).collect();
    assert_eq!(models, vec!["stub-model", "stub-model-v2"]);
}

#[tokio::test]
async fn deleting_embedding_rows_self_heals() {
    let hub = spawn().await;
    let m = hub.machine_id;
    let p = format!("/tmp/emb-{m}/heal");
    seed_session(&hub, &p, &format!("heal-{m}"), "2026-07-06").await;
    post_entry(&hub, &p, "2026-07-06", "entry", "Healing summary.").await;
    sweep(&hub.pool, &stub()).await.unwrap();
    assert_eq!(embedding_rows(&hub.pool, m).await.len(), 1);

    // Derived data: wiping it is safe and the next pass regenerates.
    sqlx::query(
        r"
        DELETE FROM journal_embeddings e
        USING journal_entries je
        WHERE je.id = e.journal_entry_id
          AND je.project_path LIKE '/tmp/emb-' || $1 || '%'
        ",
    )
    .bind(m.to_string())
    .execute(&hub.pool)
    .await
    .unwrap();
    assert_eq!(embedding_rows(&hub.pool, m).await.len(), 0);

    let stats = sweep(&hub.pool, &stub()).await.unwrap();
    assert_eq!(
        stats,
        SweepStats {
            embedded: 1,
            failed: 0
        }
    );
    assert_eq!(embedding_rows(&hub.pool, m).await.len(), 1);
}
