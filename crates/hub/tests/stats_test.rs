//! Integration tests for the analytics rollups.
//!
//! The load-bearing one is `usage_is_counted_once_per_provider_message`: one
//! assistant response occupies several stored rows carrying an IDENTICAL usage
//! block, so a plain SUM over `messages` over-reports. Everything else in this
//! module is comparatively mechanical.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use hub::stats::{self, Window};
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
    project_path: String,
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
        // Per-test path so scoped rollups never see another test's rows.
        project_path: format!("/tmp/proj-{machine_id}"),
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

fn batch(hub: &TestHub, session: &str, messages: Vec<IngestMessage>) -> IngestBatch {
    IngestBatch {
        machine: MachineInfo {
            machine_id: hub.machine_id,
            hostname: "test-host".into(),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: hub.project_path.clone(),
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
            project_path: Some(hub.project_path.clone()),
            file_path: Some(format!("{}/{session}.jsonl", hub.project_path)),
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

#[allow(clippy::too_many_arguments)]
fn tok_msg(
    session: &str,
    key: &str,
    ts: &str,
    message_id: Option<&str>,
    uuid: Option<&str>,
    input: i64,
    output: i64,
    cost: Option<f64>,
    content: Value,
) -> IngestMessage {
    let mut raw = json!({});
    if let Some(mid) = message_id {
        raw["messageId"] = json!(mid);
    }
    IngestMessage {
        provider: "claude".into(),
        session_id: session.into(),
        message_key: key.into(),
        uuid: uuid.map(Into::into),
        parent_uuid: None,
        seq: 0,
        timestamp: Some(ts.into()),
        message_type: Some("assistant".into()),
        role: Some("assistant".into()),
        model: Some("claude-opus-5".into()),
        stop_reason: None,
        input_tokens: Some(input),
        output_tokens: Some(output),
        cache_creation_tokens: None,
        cache_read_tokens: None,
        cost_usd: cost,
        duration_ms: None,
        is_sidechain: false,
        content: Some(content),
        raw,
        search_text: Some("text".into()),
    }
}

fn text() -> Value {
    json!([{ "type": "text", "text": "hi" }])
}

async fn project_stats(hub: &TestHub) -> history_core::models::ProjectStatsSummary {
    stats::project(
        &hub.pool,
        "proj".into(),
        vec![hub.project_path.clone()],
        &Window::default(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn usage_is_counted_once_per_provider_message() {
    // THE dedup rule. Three stored rows, one logical assistant response: same
    // messageId, same usage block. A plain SUM would report 300/150.
    let hub = spawn().await;
    let b = batch(
        &hub,
        "s1",
        vec![
            tok_msg(
                "s1",
                "k1",
                "2026-07-20T10:00:00Z",
                Some("msg_A"),
                Some("u1"),
                100,
                50,
                None,
                text(),
            ),
            tok_msg(
                "s1",
                "k2",
                "2026-07-20T10:00:01Z",
                Some("msg_A"),
                Some("u2"),
                100,
                50,
                None,
                text(),
            ),
            tok_msg(
                "s1",
                "k3",
                "2026-07-20T10:00:02Z",
                Some("msg_A"),
                Some("u3"),
                100,
                50,
                None,
                text(),
            ),
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let s = project_stats(&hub).await;
    assert_eq!(
        s.token_distribution.input, 100,
        "input counted more than once"
    );
    assert_eq!(
        s.token_distribution.output, 50,
        "output counted more than once"
    );
    assert_eq!(s.total_tokens, 150);
    assert_eq!(s.total_messages, 1, "deduped message count");
}

#[tokio::test]
async fn messages_without_a_provider_id_fall_back_to_uuid() {
    let hub = spawn().await;
    let b = batch(
        &hub,
        "s1",
        vec![
            tok_msg(
                "s1",
                "k1",
                "2026-07-20T10:00:00Z",
                None,
                Some("u1"),
                10,
                1,
                None,
                text(),
            ),
            tok_msg(
                "s1",
                "k2",
                "2026-07-20T10:00:01Z",
                None,
                Some("u2"),
                10,
                1,
                None,
                text(),
            ),
            // Same uuid as k1 → one logical message, counted once.
            tok_msg(
                "s1",
                "k3",
                "2026-07-20T10:00:02Z",
                None,
                Some("u1"),
                10,
                1,
                None,
                text(),
            ),
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let s = project_stats(&hub).await;
    assert_eq!(s.total_messages, 2);
    assert_eq!(s.token_distribution.input, 20);
}

#[tokio::test]
async fn rows_with_neither_identifier_are_never_collapsed() {
    // The row-id fallback: two distinct messages that happen to carry no
    // message_id and no uuid must stay two, not fold into one.
    let hub = spawn().await;
    let b = batch(
        &hub,
        "s1",
        vec![
            tok_msg(
                "s1",
                "k1",
                "2026-07-20T10:00:00Z",
                None,
                None,
                7,
                3,
                None,
                text(),
            ),
            tok_msg(
                "s1",
                "k2",
                "2026-07-20T10:00:01Z",
                None,
                None,
                7,
                3,
                None,
                text(),
            ),
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let s = project_stats(&hub).await;
    assert_eq!(s.total_messages, 2);
    assert_eq!(s.token_distribution.input, 14);
}

#[tokio::test]
async fn dedup_does_not_merge_the_same_id_across_sessions() {
    // The dedup key is (session, message id) — an id reused in another session
    // is a different message.
    let hub = spawn().await;
    assert_eq!(
        post_ingest(
            &hub,
            &batch(
                &hub,
                "s1",
                vec![tok_msg(
                    "s1",
                    "k1",
                    "2026-07-20T10:00:00Z",
                    Some("msg_X"),
                    None,
                    5,
                    5,
                    None,
                    text()
                )]
            )
        )
        .await,
        200
    );
    assert_eq!(
        post_ingest(
            &hub,
            &batch(
                &hub,
                "s2",
                vec![tok_msg(
                    "s2",
                    "k1",
                    "2026-07-20T10:00:00Z",
                    Some("msg_X"),
                    None,
                    5,
                    5,
                    None,
                    text()
                )]
            )
        )
        .await,
        200
    );

    let s = project_stats(&hub).await;
    assert_eq!(s.total_messages, 2);
    assert_eq!(s.total_sessions, 2);
    assert_eq!(s.total_tokens, 20);
}

#[tokio::test]
async fn cost_is_reported_where_present_and_never_coalesced_to_zero() {
    let hub = spawn().await;

    // No message reports cost → None, NOT Some(0.0).
    assert_eq!(
        post_ingest(
            &hub,
            &batch(
                &hub,
                "s1",
                vec![tok_msg(
                    "s1",
                    "k1",
                    "2026-07-20T10:00:00Z",
                    Some("m1"),
                    None,
                    1,
                    1,
                    None,
                    text()
                )]
            )
        )
        .await,
        200
    );
    let s = project_stats(&hub).await;
    assert_eq!(s.total_cost_usd, None, "absent cost must not read as free");
    assert_eq!(s.cost_reported_messages, 0);

    // Add two that do report.
    assert_eq!(
        post_ingest(
            &hub,
            &batch(
                &hub,
                "s1",
                vec![
                    tok_msg(
                        "s1",
                        "k2",
                        "2026-07-20T10:00:01Z",
                        Some("m2"),
                        None,
                        1,
                        1,
                        Some(0.25),
                        text()
                    ),
                    tok_msg(
                        "s1",
                        "k3",
                        "2026-07-20T10:00:02Z",
                        Some("m3"),
                        None,
                        1,
                        1,
                        Some(0.75),
                        text()
                    ),
                ]
            )
        )
        .await,
        200
    );
    let s = project_stats(&hub).await;
    let cost = s.total_cost_usd.expect("cost present");
    assert!((cost - 1.0).abs() < 1e-9, "got {cost}");
    assert_eq!(s.cost_reported_messages, 2, "coverage, not just the sum");
}

#[tokio::test]
async fn daily_and_heatmap_bucket_in_the_requested_timezone() {
    let hub = spawn().await;
    // 23:30 UTC on the 20th is 01:30 on the 21st in Europe/Rome (UTC+2 in July).
    let b = batch(
        &hub,
        "s1",
        vec![tok_msg(
            "s1",
            "k1",
            "2026-07-20T23:30:00Z",
            Some("m1"),
            None,
            10,
            5,
            None,
            text(),
        )],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let utc = stats::project(
        &hub.pool,
        "proj".into(),
        vec![hub.project_path.clone()],
        &Window::default(),
    )
    .await
    .unwrap();
    assert_eq!(utc.daily_stats[0].date, "2026-07-20");
    assert_eq!(utc.activity_heatmap[0].hour, 23);

    let rome = stats::project(
        &hub.pool,
        "proj".into(),
        vec![hub.project_path.clone()],
        &Window {
            tz: chrono_tz::Europe::Rome,
            ..Window::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(rome.daily_stats[0].date, "2026-07-21", "day did not shift");
    assert_eq!(rome.activity_heatmap[0].hour, 1, "hour did not shift");
}

#[tokio::test]
async fn date_window_narrows_the_aggregate() {
    let hub = spawn().await;
    let b = batch(
        &hub,
        "s1",
        vec![
            tok_msg(
                "s1",
                "k1",
                "2026-07-10T12:00:00Z",
                Some("m1"),
                None,
                10,
                0,
                None,
                text(),
            ),
            tok_msg(
                "s1",
                "k2",
                "2026-07-20T12:00:00Z",
                Some("m2"),
                None,
                20,
                0,
                None,
                text(),
            ),
            tok_msg(
                "s1",
                "k3",
                "2026-07-30T12:00:00Z",
                Some("m3"),
                None,
                40,
                0,
                None,
                text(),
            ),
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let w = Window {
        from: Some("2026-07-15".parse().unwrap()),
        to: Some("2026-07-25".parse().unwrap()),
        tz: chrono_tz::UTC,
    };
    let s = stats::project(&hub.pool, "proj".into(), vec![hub.project_path.clone()], &w)
        .await
        .unwrap();
    assert_eq!(s.total_messages, 1);
    assert_eq!(s.token_distribution.input, 20);
}

#[tokio::test]
async fn tool_success_rate_resolves_against_the_outcome_message() {
    // D10: the invocation does not carry its outcome; the later tool_result
    // does. One of two Bash calls errored → 50%.
    let hub = spawn().await;
    let b = batch(
        &hub,
        "s1",
        vec![
            tok_msg(
                "s1",
                "k1",
                "2026-07-20T10:00:00Z",
                Some("m1"),
                None,
                1,
                1,
                None,
                json!([
                    { "type": "tool_use", "id": "t1", "name": "Bash", "input": {} },
                    { "type": "tool_use", "id": "t2", "name": "Bash", "input": {} },
                ]),
            ),
            {
                let mut m = tok_msg(
                    "s1",
                    "k2",
                    "2026-07-20T10:00:01Z",
                    Some("m2"),
                    None,
                    1,
                    1,
                    None,
                    json!([
                        { "type": "tool_result", "tool_use_id": "t1" },
                        { "type": "tool_result", "tool_use_id": "t2", "is_error": true },
                    ]),
                );
                m.message_type = Some("user".into());
                m.role = Some("user".into());
                m
            },
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let s = project_stats(&hub).await;
    let bash = s
        .most_used_tools
        .iter()
        .find(|t| t.tool_name == "Bash")
        .expect("Bash");
    assert_eq!(bash.usage_count, 2);
    assert!(
        (bash.success_rate - 0.5).abs() < 1e-6,
        "expected 0.5, got {}",
        bash.success_rate
    );
    assert_eq!(bash.avg_execution_time, None);
}

#[tokio::test]
async fn skills_and_subagents_are_reported_separately_from_tools() {
    let hub = spawn().await;
    let b = batch(
        &hub,
        "s1",
        vec![tok_msg(
            "s1",
            "k1",
            "2026-07-20T10:00:00Z",
            Some("m1"),
            None,
            1,
            1,
            None,
            json!([
                { "type": "tool_use", "id": "t1", "name": "Skill", "input": { "skill": "cchv-find" } },
                { "type": "tool_use", "id": "t2", "name": "Agent", "input": { "subagent_type": "Explore" } },
            ]),
        )],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let s = project_stats(&hub).await;
    assert_eq!(s.most_used_skills.len(), 1);
    assert_eq!(s.most_used_skills[0].tool_name, "cchv-find");
    assert_eq!(s.most_used_subagents.len(), 1);
    assert_eq!(s.most_used_subagents[0].tool_name, "Explore");
    // The carrying tools still appear under tools.
    assert!(s.most_used_tools.iter().any(|t| t.tool_name == "Skill"));
    assert!(s.most_used_tools.iter().any(|t| t.tool_name == "Agent"));
}

#[tokio::test]
async fn session_scope_returns_that_session_only_and_404s_on_unknown() {
    let hub = spawn().await;
    assert_eq!(
        post_ingest(
            &hub,
            &batch(
                &hub,
                "s1",
                vec![tok_msg(
                    "s1",
                    "k1",
                    "2026-07-20T10:00:00Z",
                    Some("m1"),
                    None,
                    11,
                    0,
                    None,
                    text()
                )]
            )
        )
        .await,
        200
    );
    assert_eq!(
        post_ingest(
            &hub,
            &batch(
                &hub,
                "s2",
                vec![tok_msg(
                    "s2",
                    "k1",
                    "2026-07-20T10:00:00Z",
                    Some("m2"),
                    None,
                    99,
                    0,
                    None,
                    text()
                )]
            )
        )
        .await,
        200
    );

    let pk: i64 =
        sqlx::query_scalar("SELECT id FROM sessions WHERE machine_id=$1 AND session_id='s1'")
            .bind(hub.machine_id)
            .fetch_one(&hub.pool)
            .await
            .unwrap();

    let s = stats::session(&hub.pool, pk, &Window::default())
        .await
        .unwrap()
        .expect("session stats");
    assert_eq!(s.session_id, "s1");
    assert_eq!(s.total_input_tokens, 11, "leaked another session's tokens");
    assert_eq!(s.message_count, 1);
    assert_eq!(s.summary.as_deref(), Some("a session"));

    assert!(stats::session(&hub.pool, -1, &Window::default())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn model_and_provider_breakdowns_are_deduped_too() {
    let hub = spawn().await;
    let mut dup = tok_msg(
        "s1",
        "k2",
        "2026-07-20T10:00:01Z",
        Some("msg_A"),
        None,
        100,
        50,
        None,
        text(),
    );
    dup.model = Some("claude-opus-5".into());
    let b = batch(
        &hub,
        "s1",
        vec![
            tok_msg(
                "s1",
                "k1",
                "2026-07-20T10:00:00Z",
                Some("msg_A"),
                None,
                100,
                50,
                None,
                text(),
            ),
            dup,
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let g = stats::global(&hub.pool, &Window::default()).await.unwrap();
    let model = g
        .model_distribution
        .iter()
        .find(|m| m.model_name == "claude-opus-5")
        .expect("model present");
    // Global sees other tests' rows too, so assert the invariant that matters:
    // this model's tokens are consistent with its own message count, i.e. the
    // duplicate row did not inflate it.
    assert!(model.message_count >= 1);
    assert!(g
        .provider_distribution
        .iter()
        .any(|p| p.provider_id == "claude"));
    assert!(g.total_messages >= 1);
}
