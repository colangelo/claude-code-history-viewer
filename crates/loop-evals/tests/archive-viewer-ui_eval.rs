//! T2 evals for "Archive viewer UI: browse and search the hub archive from
//! the viewer" (Gitea #5).
//!
//! These drive `hub::router` directly — same harness shape as
//! `crates/hub/tests/read_test.rs::spawn()` — against a throwaway Postgres
//! (`TEST_DATABASE_URL`/`DATABASE_URL`), each test using a fresh random
//! machine/token pair for isolation in the shared test database.
//!
//! Frontend-observable criteria (hub client, settings UI, archive browser
//! component) live in `crates/loop-evals/tests/archive-viewer-ui.eval.test.tsx`.
//!
//! AC1/AC2 fail against the unmodified hub because `hub::router` has no CORS
//! layer today: a CORS preflight gets no `Access-Control-Allow-*` headers,
//! and a real GET response exposes no headers to `fetch` beyond the CORS
//! safelist (`X-Total-Count` is not one of them). AC3 fails to compile-time
//! stability but at runtime: `UserSettings` has no `archive_hub_url`/
//! `archive_hub_token` fields yet, so those keys never survive a
//! `from_value`/`to_value` round trip.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use reqwest::Method;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::sync::Arc;
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
    hostname: String,
}

async fn spawn() -> TestHub {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");

    let machine_id = Uuid::new_v4();
    let hostname = format!("host-{}", &machine_id.simple().to_string()[..12]);
    let token = format!("tok-{machine_id}");
    let mut tokens = HashMap::new();
    tokens.insert(token.clone(), machine_id);

    let state = hub::AppState {
        pool,
        tokens: Arc::new(tokens),
        trusted_identities: Arc::new(Vec::new()),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, hub::router(state, None)).await.unwrap();
    });

    TestHub {
        base: format!("http://{addr}"),
        token,
        machine_id,
        hostname,
    }
}

fn sample_batch(hub: &TestHub, session: &str) -> IngestBatch {
    IngestBatch {
        machine: MachineInfo {
            machine_id: hub.machine_id,
            hostname: hub.hostname.clone(),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: "/tmp/archive-viewer-ui".into(),
            name: Some("archive-viewer-ui".into()),
            storage_type: Some("jsonl".into()),
            session_count: Some(1),
            message_count: Some(1),
            last_modified: None,
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: session.into(),
            project_path: Some("/tmp/archive-viewer-ui".into()),
            file_path: Some(format!("/tmp/archive-viewer-ui/{session}.jsonl")),
            entrypoint: None,
            summary: Some("a session".into()),
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
            session_id: session.into(),
            message_key: "k1".into(),
            uuid: Some("u1".into()),
            parent_uuid: None,
            seq: 0,
            timestamp: Some("2026-01-01T00:00:00Z".into()),
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
            content: Some(json!([{ "type": "text", "text": "hello archive" }])),
            raw: json!({ "text": "hello archive" }),
            search_text: Some("hello archive".into()),
        }],
    }
}

async fn ingest(hub: &TestHub, batch: &IngestBatch) {
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.token)
        .json(batch)
        .send()
        .await
        .expect("send ingest request");
    assert_eq!(resp.status(), 200, "ingest setup failed");
}

/// Header value lookup that treats a missing header as `None` rather than
/// panicking, so assertion failure messages are readable.
fn header<'a>(resp: &'a reqwest::Response, name: &str) -> Option<&'a str> {
    resp.headers().get(name).and_then(|v| v.to_str().ok())
}

#[tokio::test]
async fn ac1_cors_preflight_on_projects_grants_authorization_header() {
    let hub = spawn().await;

    let resp = reqwest::Client::new()
        .request(Method::OPTIONS, format!("{}/v1/projects", hub.base))
        .header("Origin", "http://localhost:1420")
        .header("Access-Control-Request-Method", "GET")
        .header("Access-Control-Request-Headers", "authorization")
        .send()
        .await
        .expect("send preflight request");

    assert!(
        resp.status().is_success(),
        "CORS preflight must be answered with a 2xx status, got {}",
        resp.status()
    );

    let allow_origin = header(&resp, "access-control-allow-origin");
    assert!(
        allow_origin.is_some(),
        "preflight response must carry Access-Control-Allow-Origin"
    );

    let allow_headers = header(&resp, "access-control-allow-headers")
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        allow_headers == "*" || allow_headers.contains("authorization"),
        "Access-Control-Allow-Headers must grant `authorization`, got {allow_headers:?}"
    );
}

#[tokio::test]
async fn ac2_session_messages_exposes_x_total_count_via_cors() {
    let hub = spawn().await;
    // Session ids must be unique across the whole shared test database (not
    // just this test run): a fixed literal collides with the same literal
    // ingested by a prior run under a different machine, and the endpoint
    // then 400s as an ambiguous cross-machine session id instead of the 200
    // this test expects.
    let session_id = format!("sess-ac2-{}", Uuid::new_v4());
    let batch = sample_batch(&hub, &session_id);
    ingest(&hub, &batch).await;

    let resp = reqwest::Client::new()
        .get(format!("{}/v1/sessions/{session_id}/messages", hub.base))
        .header("Origin", "http://localhost:1420")
        .bearer_auth(&hub.token)
        .send()
        .await
        .expect("send session messages request");

    assert_eq!(resp.status(), 200);
    assert_eq!(
        header(&resp, "x-total-count"),
        Some("1"),
        "response must carry X-Total-Count for the session's total message count"
    );

    let allow_origin = header(&resp, "access-control-allow-origin");
    assert!(
        allow_origin.is_some(),
        "authenticated response must carry Access-Control-Allow-Origin when Origin is set"
    );

    let expose_headers = header(&resp, "access-control-expose-headers")
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        expose_headers == "*" || expose_headers.contains("x-total-count"),
        "Access-Control-Expose-Headers must grant `x-total-count` so `fetch` can read it, got {expose_headers:?}"
    );
}

#[tokio::test]
async fn ac3_user_settings_archive_hub_fields_roundtrip_dynamically() {
    // Dynamic surface only, per the loop's compile-against-unmodified-crate
    // rule: never reference new struct fields directly, round-trip through
    // serde_json::Value instead.
    let with_hub = json!({
        "archiveHubUrl": "http://h:8787",
        "archiveHubToken": "tok",
    });
    let settings: history_core::models::UserSettings =
        serde_json::from_value(with_hub).expect("deserialize UserSettings with hub fields");
    let round_tripped: Value =
        serde_json::to_value(&settings).expect("serialize UserSettings back to JSON");

    assert_eq!(
        round_tripped.get("archiveHubUrl"),
        Some(&json!("http://h:8787")),
        "archiveHubUrl must survive the round trip with its exact value, got {round_tripped}"
    );
    assert_eq!(
        round_tripped.get("archiveHubToken"),
        Some(&json!("tok")),
        "archiveHubToken must survive the round trip with its exact value, got {round_tripped}"
    );

    // Absent from an empty settings object -> absent from serialized output.
    let empty: history_core::models::UserSettings =
        serde_json::from_value(json!({})).expect("deserialize empty UserSettings");
    let empty_json: Value = serde_json::to_value(&empty).expect("serialize empty UserSettings");
    assert!(
        empty_json.get("archiveHubUrl").is_none(),
        "archiveHubUrl must be omitted (skip_serializing_if) when unset, got {empty_json}"
    );
    assert!(
        empty_json.get("archiveHubToken").is_none(),
        "archiveHubToken must be omitted (skip_serializing_if) when unset, got {empty_json}"
    );
}
