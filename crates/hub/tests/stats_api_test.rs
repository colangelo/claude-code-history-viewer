//! Integration tests for the `/v1/stats/*` endpoints: auth, parameter
//! handling, identity folding across machines, and both not-found cases.
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
    pool: PgPool,
    /// Two machines, so identity folding has something to fold.
    machines: Vec<Uuid>,
    root_commit: String,
    /// Run-unique path prefix. Identity expansion matches by PATH, not just by
    /// key, so a generic path would fold in rows written by other tests sharing
    /// the database — the isolation bug this prefix exists to prevent.
    prefix: String,
}

async fn spawn() -> TestHub {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");

    let machines: Vec<Uuid> = vec![Uuid::new_v4(), Uuid::new_v4()];
    let token = format!("tok-{}", machines[0]);
    let mut tokens = HashMap::new();
    for m in &machines {
        tokens.insert(format!("tok-{m}"), *m);
    }

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
        pool,
        // Identity keys are global, so a per-run root keeps runs from colliding.
        root_commit: format!("{}00000000", machines[0].simple()),
        prefix: format!("/tmp/stats-api-{}", machines[0].simple()),
        machines,
    }
}

async fn get(hub: &TestHub, path: &str, token: Option<&str>) -> (reqwest::StatusCode, Value) {
    let mut req = reqwest::Client::new().get(format!("{}{}", hub.base, path));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.expect("request");
    let status = resp.status();
    let body = resp.json::<Value>().await.unwrap_or(Value::Null);
    (status, body)
}

fn msg(session: &str, key: &str, ts: &str, mid: &str, input: i64) -> IngestMessage {
    IngestMessage {
        provider: "claude".into(),
        session_id: session.into(),
        message_key: key.into(),
        uuid: Some(format!("uuid-{key}")),
        parent_uuid: None,
        seq: 0,
        timestamp: Some(ts.into()),
        message_type: Some("assistant".into()),
        role: Some("assistant".into()),
        model: Some("claude-opus-5".into()),
        stop_reason: None,
        input_tokens: Some(input),
        output_tokens: Some(0),
        cache_creation_tokens: None,
        cache_read_tokens: None,
        cost_usd: None,
        duration_ms: None,
        is_sidechain: false,
        content: Some(json!([{ "type": "text", "text": "hi" }])),
        raw: json!({ "messageId": mid }),
        search_text: Some("text".into()),
    }
}

/// Ingest one session on `machine`, at `path`, fingerprinted to the shared root
/// commit so both machines' rows fold into one identity.
async fn ingest_on(hub: &TestHub, machine: Uuid, path: &str, session: &str, input: i64) {
    let batch = IngestBatch {
        machine: MachineInfo {
            machine_id: machine,
            hostname: format!("host-{machine}"),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: path.into(),
            name: Some("shared-proj".into()),
            storage_type: Some("jsonl".into()),
            session_count: Some(1),
            message_count: Some(1),
            last_modified: None,
            git_root_commit: Some(hub.root_commit.clone()),
            ..Default::default()
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: session.into(),
            project_path: Some(path.into()),
            file_path: Some(format!("{path}/{session}.jsonl")),
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
        messages: vec![msg(
            session,
            "k1",
            "2026-07-20T10:00:00Z",
            &format!("m-{session}"),
            input,
        )],
    };
    let status = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(format!("tok-{machine}"))
        .json(&batch)
        .send()
        .await
        .expect("ingest")
        .status();
    assert_eq!(status, 200, "ingest failed for {machine}");
}

#[tokio::test]
async fn stats_endpoints_require_a_valid_token() {
    let hub = spawn().await;
    for path in [
        "/v1/stats/global",
        "/v1/stats/projects/anything",
        "/v1/stats/sessions/1",
    ] {
        let (status, _) = get(&hub, path, None).await;
        assert_eq!(status, 401, "{path} without a token");
        let (status, _) = get(&hub, path, Some("nope")).await;
        assert_eq!(status, 401, "{path} with a bad token");
    }
}

#[tokio::test]
async fn global_returns_the_full_summary_shape() {
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/g1", hub.prefix),
        "gs1",
        42,
    )
    .await;

    let (status, body) = get(&hub, "/v1/stats/global", Some(&hub.token)).await;
    assert_eq!(status, 200);
    for key in [
        "total_projects",
        "total_sessions",
        "total_messages",
        "total_tokens",
        "date_range",
        "token_distribution",
        "daily_stats",
        "activity_heatmap",
        "most_used_tools",
        "most_used_skills",
        "most_used_subagents",
        "provider_distribution",
        "model_distribution",
        "top_projects",
    ] {
        assert!(body.get(key).is_some(), "missing {key} in global summary");
    }
    assert!(body["total_messages"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn project_stats_fold_across_machines_and_paths() {
    // The identity point (D6): the same repo on two machines at two paths must
    // report as ONE project, not two.
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/repo", hub.prefix),
        "s-a",
        100,
    )
    .await;
    ingest_on(
        &hub,
        hub.machines[1],
        &format!("{}/elsewhere/repo", hub.prefix),
        "s-b",
        200,
    )
    .await;

    let key: String =
        sqlx::query_scalar("SELECT identity_key FROM projects WHERE git_root_commit = $1 LIMIT 1")
            .bind(&hub.root_commit)
            .fetch_one(&hub.pool)
            .await
            .expect("identity derived");

    let (status, body) = get(&hub, &format!("/v1/stats/projects/{key}"), Some(&hub.token)).await;
    assert_eq!(status, 200);
    assert_eq!(
        body["total_sessions"].as_u64().unwrap(),
        2,
        "sessions not folded"
    );
    assert_eq!(body["total_messages"].as_u64().unwrap(), 2);
    assert_eq!(
        body["token_distribution"]["input"].as_u64().unwrap(),
        300,
        "tokens not summed across machines"
    );
    assert_eq!(body["project_name"].as_str().unwrap(), "shared-proj");
}

#[tokio::test]
async fn unknown_identity_and_session_are_404_not_empty_success() {
    let hub = spawn().await;
    let (status, _) = get(
        &hub,
        "/v1/stats/projects/no-such-identity",
        Some(&hub.token),
    )
    .await;
    assert_eq!(status, 404, "unknown identity must not return zeroed stats");

    let (status, _) = get(&hub, "/v1/stats/sessions/999999999", Some(&hub.token)).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn session_stats_return_that_session() {
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/sess", hub.prefix),
        "only",
        77,
    )
    .await;
    let pk: i64 =
        sqlx::query_scalar("SELECT id FROM sessions WHERE machine_id = $1 AND session_id = 'only'")
            .bind(hub.machines[0])
            .fetch_one(&hub.pool)
            .await
            .unwrap();

    let (status, body) = get(&hub, &format!("/v1/stats/sessions/{pk}"), Some(&hub.token)).await;
    assert_eq!(status, 200);
    assert_eq!(body["session_id"].as_str().unwrap(), "only");
    assert_eq!(body["total_input_tokens"].as_u64().unwrap(), 77);
    assert_eq!(body["message_count"].as_u64().unwrap(), 1);
}

#[tokio::test]
async fn date_window_narrows_and_is_validated() {
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/win", hub.prefix),
        "ws",
        55,
    )
    .await; // 2026-07-20

    let key: String =
        sqlx::query_scalar("SELECT identity_key FROM projects WHERE git_root_commit = $1 LIMIT 1")
            .bind(&hub.root_commit)
            .fetch_one(&hub.pool)
            .await
            .unwrap();

    // Window containing the message.
    let (status, body) = get(
        &hub,
        &format!("/v1/stats/projects/{key}?from=2026-07-19&to=2026-07-21"),
        Some(&hub.token),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["total_messages"].as_u64().unwrap(), 1);

    // Window excluding it.
    let (status, body) = get(
        &hub,
        &format!("/v1/stats/projects/{key}?from=2026-07-01&to=2026-07-02"),
        Some(&hub.token),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["total_messages"].as_u64().unwrap(), 0);

    // Malformed and contradictory parameters are 400s that name the problem,
    // not 500s from the database.
    for bad in [
        "?from=yesterday",
        "?to=2026-13-45",
        "?from=2026-07-20&to=2026-07-01",
        "?tz=Not/A/Zone;DROP",
    ] {
        let (status, _) = get(
            &hub,
            &format!("/v1/stats/projects/{key}{bad}"),
            Some(&hub.token),
        )
        .await;
        assert_eq!(status, 400, "expected 400 for {bad}");
    }
}

#[tokio::test]
async fn timezone_parameter_shifts_the_buckets() {
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/tz", hub.prefix),
        "tzs",
        5,
    )
    .await; // 10:00Z

    let key: String =
        sqlx::query_scalar("SELECT identity_key FROM projects WHERE git_root_commit = $1 LIMIT 1")
            .bind(&hub.root_commit)
            .fetch_one(&hub.pool)
            .await
            .unwrap();

    let (_, utc) = get(&hub, &format!("/v1/stats/projects/{key}"), Some(&hub.token)).await;
    let (_, rome) = get(
        &hub,
        &format!("/v1/stats/projects/{key}?tz=Europe/Rome"),
        Some(&hub.token),
    )
    .await;

    assert_eq!(utc["activity_heatmap"][0]["hour"].as_u64().unwrap(), 10);
    assert_eq!(
        rome["activity_heatmap"][0]["hour"].as_u64().unwrap(),
        12,
        "UTC+2 in July"
    );
}
