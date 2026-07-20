//! Integration tests for `/v1/search` journal modes (task group 3):
//! keyword byte-compatibility, semantic paraphrase recall via contrived stub
//! vectors, hybrid fusion, graceful degradation, and unknown-mode rejection.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).
//! Every query here is project-scoped to this test's paths: the shared test
//! database holds other rows (and other tests sweep them into the same stub
//! model space), so scoping is what isolates the assertions.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use uuid::Uuid;

use hub::embed::{query_text, Embedder, StubEmbedder};
use hub::embed_sweep::{embed_text, sweep};

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

async fn spawn(embedder: Option<Arc<dyn Embedder>>) -> TestHub {
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

    let mut state = hub::AppState::new(pool.clone(), tokens, Vec::new());
    if let Some(e) = embedder {
        state = state.with_embedder(e);
    }
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

fn enc(s: &str) -> String {
    s.replace('%', "%25")
        .replace('&', "%26")
        .replace('+', "%2B")
        .replace(':', "%3A")
        .replace('/', "%2F")
        .replace(' ', "%20")
}

async fn get_raw(hub: &TestHub, path_and_query: &str) -> (u16, String) {
    let resp = client()
        .get(format!("{}{path_and_query}", hub.base))
        .bearer_auth(&hub.token)
        .send()
        .await
        .expect("request");
    let status = resp.status().as_u16();
    (status, resp.text().await.expect("body"))
}

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

async fn post_entry(hub: &TestHub, path: &str, date: &str, headline: &str, summary: &str) {
    let resp = client()
        .get(format!("{}/v1/sessions?project={}", hub.base, enc(path)))
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap();
    let sessions: Value = resp.json().await.unwrap();
    // Provenance must exactly cover the (date, project) group: only this
    // date's sessions (seeded at noon UTC, so calendar date == logical date).
    let ids: Vec<i64> = sessions
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| {
            s["first_message_time"]
                .as_str()
                .is_some_and(|t| t.starts_with(date))
        })
        .map(|s| s["id"].as_i64().unwrap())
        .collect();
    assert!(!ids.is_empty());
    let resp = client()
        .post(format!("{}/v1/journal/entries", hub.base))
        .bearer_auth(&hub.token)
        .json(&json!({
            "entry_date": date,
            "project_path": path,
            "status": "entry",
            "headline": headline,
            "summary": summary,
            "topics": ["one", "two", "three"],
            "open_questions": [],
            "session_ids": ids,
            "model": "test-model",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "journal entry create");
}

/// The exact text the sweep embeds for entries posted via [`post_entry`].
fn entry_text(headline: &str, summary: &str) -> String {
    embed_text(
        Some(headline),
        Some(summary),
        &["one".into(), "two".into(), "three".into()],
        &[],
    )
}

#[tokio::test]
async fn keyword_mode_is_byte_compatible() {
    let hub = spawn(None).await;
    let m = hub.machine_id;
    let p = format!("/tmp/mode-{m}/compat");
    seed_session(&hub, &p, &format!("compat-{m}"), "2026-07-01").await;
    post_entry(
        &hub,
        &p,
        "2026-07-01",
        "Compat headline",
        "Compat haystack summary.",
    )
    .await;

    let q = format!("/v1/search?q=haystack&scope=journal&project={}", enc(&p));
    let (status_default, body_default) = get_raw(&hub, &q).await;
    let (status_keyword, body_keyword) = get_raw(&hub, &format!("{q}&mode=keyword")).await;
    assert_eq!(status_default, 200);
    assert_eq!(status_keyword, 200);
    // `mode=keyword` and no-mode are the same bytes, and neither leaks the
    // degradation field.
    assert_eq!(body_default, body_keyword);
    assert!(!body_default.contains("journal_degraded"));
    let v: Value = serde_json::from_str(&body_default).unwrap();
    assert_eq!(v["journal"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn semantic_mode_surfaces_paraphrase_the_fts_misses() {
    // Contrived vectors: the paraphrase query points at the chezmoi entry,
    // orthogonal to the unrelated one. No vocabulary is shared with either.
    let target_text = entry_text(
        "Chezmoi dotfiles drift aligned",
        "Aligned chezmoi source repos across machines.",
    );
    let other_text = entry_text(
        "Postgres pool sizing",
        "Tuned hub pool min connections for resilience.",
    );
    let paraphrase = "keeping config files identical on both laptops";
    let stub: Arc<dyn Embedder> = Arc::new(
        StubEmbedder::new("stub-model", 4)
            .with(&target_text, vec![1.0, 0.0, 0.0, 0.0])
            .with(&other_text, vec![0.0, 1.0, 0.0, 0.0])
            .with(&query_text(paraphrase), vec![0.95, 0.05, 0.0, 0.0]),
    );
    let hub = spawn(Some(stub.clone())).await;
    let m = hub.machine_id;
    let p = format!("/tmp/mode-{m}/sem");
    seed_session(&hub, &p, &format!("sem1-{m}"), "2026-07-02").await;
    seed_session(&hub, &p, &format!("sem2-{m}"), "2026-07-03").await;
    post_entry(
        &hub,
        &p,
        "2026-07-02",
        "Chezmoi dotfiles drift aligned",
        "Aligned chezmoi source repos across machines.",
    )
    .await;
    post_entry(
        &hub,
        &p,
        "2026-07-03",
        "Postgres pool sizing",
        "Tuned hub pool min connections for resilience.",
    )
    .await;
    sweep(&hub.pool, stub.as_ref()).await.unwrap();

    // The measured gap: FTS finds nothing for the paraphrase…
    let (status, body) = get_raw(
        &hub,
        &format!(
            "/v1/search?q={}&scope=journal&project={}",
            enc(paraphrase),
            enc(&p)
        ),
    )
    .await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["journal"].as_array().unwrap().len(), 0, "keyword misses");

    // …while semantic surfaces the right entry first, not degraded.
    let (status, body) = get_raw(
        &hub,
        &format!(
            "/v1/search?q={}&scope=journal&project={}&mode=semantic",
            enc(paraphrase),
            enc(&p)
        ),
    )
    .await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(v.get("journal_degraded").is_none());
    let hits = v["journal"].as_array().unwrap();
    assert_eq!(hits.len(), 2, "both scoped entries ranked");
    assert_eq!(hits[0]["headline"], json!("Chezmoi dotfiles drift aligned"));
    assert!(hits[0]["rank"].as_f64().unwrap() > hits[1]["rank"].as_f64().unwrap());
}

#[tokio::test]
async fn hybrid_mode_fuses_keyword_and_semantic() {
    // Entry A is a pure KEYWORD hit ("flargle" appears verbatim; its vector is
    // orthogonal to the query). Entry B is a pure SEMANTIC hit (no shared
    // vocabulary; vector aligned with the query). Hybrid must surface both.
    let a_text = entry_text("Flargle refactor", "The flargle subsystem was refactored.");
    let b_text = entry_text(
        "Deploy relay hardening",
        "Made the infra handoff resilient to retries.",
    );
    let query = "flargle rollout reliability";
    let stub: Arc<dyn Embedder> = Arc::new(
        StubEmbedder::new("stub-model", 4)
            .with(&a_text, vec![0.0, 1.0, 0.0, 0.0])
            .with(&b_text, vec![1.0, 0.0, 0.0, 0.0])
            .with(&query_text(query), vec![1.0, 0.0, 0.0, 0.0]),
    );
    let hub = spawn(Some(stub.clone())).await;
    let m = hub.machine_id;
    let p = format!("/tmp/mode-{m}/hyb");
    seed_session(&hub, &p, &format!("hyb1-{m}"), "2026-07-04").await;
    seed_session(&hub, &p, &format!("hyb2-{m}"), "2026-07-05").await;
    post_entry(
        &hub,
        &p,
        "2026-07-04",
        "Flargle refactor",
        "The flargle subsystem was refactored.",
    )
    .await;
    post_entry(
        &hub,
        &p,
        "2026-07-05",
        "Deploy relay hardening",
        "Made the infra handoff resilient to retries.",
    )
    .await;
    sweep(&hub.pool, stub.as_ref()).await.unwrap();

    let (status, body) = get_raw(
        &hub,
        &format!(
            "/v1/search?q={}&scope=journal&project={}&mode=hybrid",
            enc(query),
            enc(&p)
        ),
    )
    .await;
    assert_eq!(status, 200);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(v.get("journal_degraded").is_none());
    let headlines: Vec<&str> = v["journal"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["headline"].as_str().unwrap())
        .collect();
    assert!(
        headlines.contains(&"Flargle refactor"),
        "keyword-only hit fused in: {headlines:?}"
    );
    assert!(
        headlines.contains(&"Deploy relay hardening"),
        "semantic-only hit fused in: {headlines:?}"
    );
}

#[tokio::test]
async fn semantic_without_embedder_degrades_to_keyword() {
    let hub = spawn(None).await;
    let m = hub.machine_id;
    let p = format!("/tmp/mode-{m}/deg");
    seed_session(&hub, &p, &format!("deg-{m}"), "2026-07-06").await;
    post_entry(
        &hub,
        &p,
        "2026-07-06",
        "Degraded headline",
        "Degraded needle summary.",
    )
    .await;

    let (status, body) = get_raw(
        &hub,
        &format!(
            "/v1/search?q=needle&scope=journal&project={}&mode=hybrid",
            enc(&p)
        ),
    )
    .await;
    assert_eq!(status, 200, "degradation is never an error");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["journal_degraded"], json!(true));
    assert_eq!(
        v["journal"].as_array().unwrap().len(),
        1,
        "keyword results still served"
    );
}

#[tokio::test]
async fn unknown_mode_is_rejected() {
    let hub = spawn(None).await;
    let (status, body) = get_raw(&hub, "/v1/search?q=x&mode=cosmic").await;
    assert_eq!(status, 400);
    assert!(body.contains("unknown mode"));
}
