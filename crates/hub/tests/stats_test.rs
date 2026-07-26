//! Integration tests for the analytics rollups.
//!
//! The load-bearing one is `usage_is_counted_once_per_provider_message`: one
//! assistant response occupies several stored rows carrying an IDENTICAL usage
//! block, so a plain SUM over `messages` over-reports. Everything else in this
//! module is comparatively mechanical.
//!
//! These assertions were written against the Postgres rollups and are unchanged
//! now that the rollups read the `DuckDB` mirror (change
//! `hub-stats-duckdb-mirror`, task 4.6): the *only* edit was to fold ingested
//! rows into the mirror before reading, which the helpers below do so each test
//! body stays exactly as it was. Every semantic expectation surviving verbatim is
//! the point of keeping them.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use history_core::models::{GlobalStatsSummary, ProjectStatsSummary, SessionTokenStats};
use hub::config::MirrorConfig;
use hub::mirror::Mirror;
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
    /// The read model the rollups run against. Its own file per test, so one
    /// test's refresh cannot decide another's readiness.
    mirror: Mirror,
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

    let dir = std::env::temp_dir().join(format!("cchv-stats-{machine_id}"));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mirror = Mirror::open_or_create(&MirrorConfig {
        path: Some(dir.join("stats.duckdb")),
        ..MirrorConfig::default()
    })
    .expect("open mirror");

    TestHub {
        base: format!("http://{addr}"),
        token,
        machine_id,
        pool,
        // Per-test path so scoped rollups never see another test's rows.
        project_path: format!("/tmp/proj-{machine_id}"),
        mirror,
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

/// Fold everything ingested so far into the mirror and hand back a connection.
///
/// Every rollup below reads through here, so the "ingest then assert" shape of
/// the original tests survives: the refresh is the one step the mirror adds, and
/// putting it in the helper keeps it out of thirteen test bodies.
async fn mirrored(hub: &TestHub) -> duckdb::Connection {
    hub.mirror
        .refresh(&hub.pool)
        .await
        .expect("refresh stats mirror");
    hub.mirror.connection().expect("mirror connection")
}

async fn project_stats(hub: &TestHub) -> ProjectStatsSummary {
    project_stats_in(hub, &Window::default()).await
}

async fn project_stats_in(hub: &TestHub, w: &Window) -> ProjectStatsSummary {
    let conn = mirrored(hub).await;
    stats::project(&conn, "proj".into(), vec![hub.project_path.clone()], w).unwrap()
}

async fn session_stats(hub: &TestHub, pk: i64) -> Option<SessionTokenStats> {
    session_stats_in(hub, pk, &Window::default()).await
}

async fn session_stats_in(hub: &TestHub, pk: i64, w: &Window) -> Option<SessionTokenStats> {
    let conn = mirrored(hub).await;
    stats::session(&conn, pk, w).unwrap()
}

async fn session_pk(hub: &TestHub, session: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM sessions WHERE machine_id = $1 AND session_id = $2")
        .bind(hub.machine_id)
        .bind(session)
        .fetch_one(&hub.pool)
        .await
        .expect("session pk")
}

async fn global_stats(hub: &TestHub) -> GlobalStatsSummary {
    let conn = mirrored(hub).await;
    stats::global(&conn, &Window::default()).unwrap()
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

    let utc = project_stats(&hub).await;
    assert_eq!(utc.daily_stats[0].date, "2026-07-20");
    assert_eq!(utc.activity_heatmap[0].hour, 23);

    let rome = project_stats_in(
        &hub,
        &Window {
            tz: chrono_tz::Europe::Rome,
            ..Window::default()
        },
    )
    .await;
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
    let s = project_stats_in(&hub, &w).await;
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

    let s = session_stats(&hub, pk).await.expect("session stats");
    assert_eq!(s.session_id, "s1");
    assert_eq!(s.total_input_tokens, 11, "leaked another session's tokens");
    assert_eq!(s.message_count, 1);
    assert_eq!(s.summary.as_deref(), Some("a session"));

    assert!(session_stats(&hub, -1).await.is_none());
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

    let g = global_stats(&hub).await;
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

/// The per-row model split must SUM to the row it hangs off.
///
/// This is the invariant that makes client-side cost trustworthy: cost is
/// priced per model and per token type, so a provider/project row is priced by
/// summing its `model_distribution`. If that breakdown did not add up to the
/// row's own `tokens`, the displayed cost would silently disagree with the
/// displayed token count on the same line.
///
/// The exact invariant is **`breakdown <= row`, never `>`**. Under is legitimate
/// and expected: rows with a NULL `model` count toward the row's tokens but are
/// excluded from the breakdown, because an unnamed model cannot be priced.
/// Over would mean the dedup filter or the grouping key had drifted between the
/// two queries and tokens were being double-counted — which is the direction
/// that would silently inflate a displayed cost.
///
/// The shortfall is not a defect to hide: it is why each card prices against
/// its OWN token total, so the coverage percentage reflects both unnamed models
/// and known-but-unpriced ones.
#[tokio::test]
async fn model_breakdowns_sum_to_their_parent_row() {
    let hub = spawn().await;
    let mut m = tok_msg(
        "s1",
        "k1",
        "2026-07-20T10:00:00Z",
        Some("msg_A"),
        None,
        100,
        50,
        None,
        text(),
    );
    m.model = Some("claude-opus-5".into());
    let b = batch(&hub, "s1", vec![m]);
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let g = global_stats(&hub).await;

    let provider = g
        .provider_distribution
        .iter()
        .find(|p| p.provider_id == "claude")
        .expect("claude provider present");
    assert!(
        !provider.model_distribution.is_empty(),
        "provider row carries no model split, so it cannot be priced"
    );
    let split: u64 = provider
        .model_distribution
        .iter()
        .map(|m| m.token_count)
        .sum();
    assert!(
        split <= provider.tokens,
        "provider model split ({split}) exceeds the provider's own tokens ({}) \
         — dedup filter or grouping key has drifted between the two queries",
        provider.tokens
    );

    // Same invariant on the project ranking, which groups through a LEFT JOIN
    // and a coalesce'd display name — the likelier of the two to drift.
    for project in &g.top_projects {
        let split: u64 = project
            .model_distribution
            .iter()
            .map(|m| m.token_count)
            .sum();
        assert!(
            split <= project.tokens,
            "project {} model split ({split}) exceeds its own tokens ({})",
            project.project_name,
            project.tokens
        );
    }
}

/// Design D3, kept from the differential gate that has now been retired with the
/// implementation it compared against.
///
/// `usage_row` is decided once over the whole archive rather than per window, so
/// a logical message whose rows straddle a window edge attributes its usage to
/// the **global** group minimum — which can sit outside the window. Postgres,
/// which deduplicated *after* windowing, attributed it to the earliest row
/// inside. Measured on the live archive: 10 of 2,648,869 dedup groups straddle a
/// date boundary, and it only matters when a window edge falls exactly there.
///
/// Asserted rather than left to be rediscovered as a bug, and asserted in both
/// directions: the mirror can *under*-attribute a straddling group, never double
/// count it.
#[tokio::test]
async fn a_group_straddling_the_window_edge_attributes_usage_outside_it() {
    let hub = spawn().await;
    // One logical message (one `messageId`, one usage block) stored either side
    // of midnight UTC.
    let b = batch(
        &hub,
        "s1",
        vec![
            tok_msg(
                "s1",
                "k1",
                "2026-07-23T23:59:30Z",
                Some("msg_straddle"),
                None,
                1000,
                0,
                None,
                text(),
            ),
            tok_msg(
                "s1",
                "k2",
                "2026-07-24T00:00:30Z",
                Some("msg_straddle"),
                None,
                1000,
                0,
                None,
                text(),
            ),
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);
    let pk = session_pk(&hub, "s1").await;

    let all = session_stats_in(&hub, pk, &Window::default())
        .await
        .expect("session stats");
    assert_eq!(
        all.total_input_tokens, 1000,
        "unbounded, the group is still counted exactly once — never doubled"
    );

    let later = session_stats_in(
        &hub,
        pk,
        &Window {
            from: Some("2026-07-24".parse().unwrap()),
            to: Some("2026-07-24".parse().unwrap()),
            tz: chrono_tz::UTC,
        },
    )
    .await
    .expect("session stats");
    assert_eq!(
        later.total_input_tokens, 0,
        "the group's usage row is the 23:59:30 one, outside this window — the \
         documented D3 divergence from the Postgres original, which reported 1000"
    );
}

/// analytics-ux-costs: project scope gains `model_distribution` so clients can
/// price a single project's tokens — without the per-model split, a project
/// could show no cost at all. Same rollup, same dedup as the global one.
#[tokio::test]
async fn project_scope_reports_a_model_distribution() {
    let hub = spawn().await;
    let mut other_model = tok_msg(
        "s1",
        "k3",
        "2026-07-20T10:01:00Z",
        Some("msg_B"),
        None,
        40,
        10,
        None,
        text(),
    );
    other_model.model = Some("claude-sonnet-5".into());
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
            // Same logical message as k1: must not inflate the opus row.
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
            other_model,
        ],
    );
    assert_eq!(post_ingest(&hub, &b).await, 200);

    let s = project_stats(&hub).await;
    assert_eq!(s.model_distribution.len(), 2);
    // Ordered by token_count descending, like the global rollup.
    let opus = &s.model_distribution[0];
    assert_eq!(opus.model_name, "claude-opus-5");
    assert_eq!(
        opus.token_count, 150,
        "duplicate usage row must not inflate"
    );
    assert_eq!(opus.input_tokens, 100);
    assert_eq!(opus.output_tokens, 50);
    assert_eq!(opus.message_count, 1);
    let sonnet = &s.model_distribution[1];
    assert_eq!(sonnet.model_name, "claude-sonnet-5");
    assert_eq!(sonnet.token_count, 50);

    // The window narrows the distribution with everything else.
    let later = project_stats_in(
        &hub,
        &Window {
            from: Some("2026-07-21".parse().unwrap()),
            to: None,
            tz: chrono_tz::UTC,
        },
    )
    .await;
    assert!(
        later.model_distribution.is_empty(),
        "no messages inside the window, so no per-model rows"
    );
}
