//! The differential gate — change `hub-stats-duckdb-mirror`, task 4.5.
//!
//! Runs the outgoing Postgres rollups and the incoming `DuckDB` ones over the
//! **same rows** and requires their serialized output to be equal field for
//! field. This catches porting errors far more directly than the desktop oracle
//! can: the oracle exists to catch semantic drift against an *independent*
//! implementation, whereas this compares against the very code being replaced.
//!
//! Design D4 makes this a gate, not a nicety — the Postgres implementation is
//! deleted rather than kept as a fallback, so this is the last moment the two
//! can be compared at all.
//!
//! Comparison is done on the serde representation rather than field by field in
//! Rust: it covers every field automatically, so a field added later is
//! compared without anyone remembering to update this file.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use hub::config::MirrorConfig;
use hub::mirror::Mirror;
use hub::stats::Window;
use hub::{stats, stats_duck};
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

struct Fixture {
    pool: PgPool,
    mirror: Mirror,
    project_path: String,
    session_pk: i64,
}

async fn spawn_and_seed() -> Fixture {
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
    let base = format!("http://{addr}");
    let project_path = format!("/tmp/diff-{machine_id}");

    // A fixture that reaches every rollup rather than just the easy ones:
    // duplicate usage blocks (dedup), a non-conversational record, two models,
    // tool/skill/subagent invocations with mixed outcomes, two sessions, and
    // timestamps spread across days and hours so daily/heatmap are non-trivial.
    let batches = vec![
        seed_batch(
            machine_id,
            &project_path,
            "sd1",
            vec![
                msg("sd1", "a1", "2026-07-20T09:15:00Z", Some("msg_A"), 100, 50)
                    .model("claude-opus-5"),
                // Same logical message: usage must count once, tools twice.
                msg("sd1", "a2", "2026-07-20T09:15:01Z", Some("msg_A"), 100, 50)
                    .model("claude-opus-5")
                    .content(json!([
                        { "type": "tool_use", "id": "t1", "name": "Bash", "input": {} },
                        { "type": "tool_use", "id": "t2", "name": "Read", "input": {} }
                    ])),
                msg("sd1", "a3", "2026-07-20T09:15:02Z", Some("msg_A"), 100, 50)
                    .model("claude-opus-5"),
                // The outcomes, on a later record (design D10).
                msg("sd1", "a4", "2026-07-20T09:16:00Z", Some("msg_B"), 5, 5)
                    .model("claude-opus-5")
                    .content(json!([
                        { "type": "tool_result", "tool_use_id": "t1" },
                        { "type": "tool_result", "tool_use_id": "t2", "is_error": true }
                    ])),
                // Bookkeeping record: no role, so not conversational.
                msg("sd1", "a5", "2026-07-20T09:17:00Z", Some("msg_C"), 7, 3).no_role(),
                msg("sd1", "a6", "2026-07-21T23:40:00Z", Some("msg_D"), 11, 13)
                    .model("claude-sonnet-5")
                    .cost(0.004_25),
            ],
        ),
        seed_batch(
            machine_id,
            &project_path,
            "sd2",
            vec![
                msg("sd2", "b1", "2026-07-22T02:05:00Z", Some("msg_E"), 20, 30)
                    .model("claude-sonnet-5")
                    .content(json!([
                        { "type": "tool_use", "id": "t3", "name": "Skill",
                          "input": { "skill": "cchv-find" } },
                        { "type": "tool_use", "id": "t4", "name": "Agent",
                          "input": { "subagent_type": "Explore" } }
                    ])),
                // Within the 30-minute idle cap: contributes to active time.
                msg("sd2", "b2", "2026-07-22T02:20:00Z", Some("msg_F"), 1, 1)
                    .model("claude-sonnet-5"),
                // Beyond it: must NOT contribute (the gap is discarded).
                msg("sd2", "b3", "2026-07-22T09:00:00Z", Some("msg_G"), 2, 2)
                    .model("claude-sonnet-5"),
                // No message id and no uuid collision: its own dedup group.
                msg("sd2", "b4", "2026-07-22T09:01:00Z", None, 3, 4).model("claude-opus-5"),
            ],
        ),
    ];
    for b in batches {
        let status = reqwest::Client::new()
            .post(format!("{base}/v1/ingest"))
            .bearer_auth(&token)
            .json(&b)
            .send()
            .await
            .expect("post ingest")
            .status();
        assert_eq!(status, 200, "ingest failed");
    }

    let session_pk: i64 =
        sqlx::query_scalar("SELECT id FROM sessions WHERE machine_id = $1 AND session_id = 'sd1'")
            .bind(machine_id)
            .fetch_one(&pool)
            .await
            .expect("session pk");

    let dir = std::env::temp_dir().join(format!("cchv-diff-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let cfg = MirrorConfig {
        path: Some(dir.join("stats.duckdb")),
        ..MirrorConfig::default()
    };
    let mirror = Mirror::open_or_create(&cfg).expect("open mirror");
    mirror.refresh(&pool).await.expect("refresh mirror");

    Fixture {
        pool,
        mirror,
        project_path,
        session_pk,
    }
}

// --- fixture builders -------------------------------------------------------

struct Msg(IngestMessage);

impl Msg {
    fn model(mut self, m: &str) -> Self {
        self.0.model = Some(m.into());
        self
    }
    fn cost(mut self, c: f64) -> Self {
        self.0.cost_usd = Some(c);
        self
    }
    fn content(mut self, c: Value) -> Self {
        self.0.content = Some(c);
        self
    }
    /// A stored record that is not a conversational turn (mode changes,
    /// attachments, custom titles). `role` is what distinguishes them.
    fn no_role(mut self) -> Self {
        self.0.role = None;
        self.0.message_type = Some("attachment".into());
        self
    }
}

fn msg(
    session: &str,
    key: &str,
    ts: &str,
    message_id: Option<&str>,
    input: i64,
    output: i64,
) -> Msg {
    let mut raw = json!({});
    if let Some(mid) = message_id {
        raw["messageId"] = json!(mid);
    }
    Msg(IngestMessage {
        provider: "claude".into(),
        session_id: session.into(),
        message_key: key.into(),
        uuid: Some(Uuid::new_v4().to_string()),
        parent_uuid: None,
        seq: 0,
        timestamp: Some(ts.into()),
        message_type: Some("assistant".into()),
        role: Some("assistant".into()),
        model: Some("claude-opus-5".into()),
        stop_reason: None,
        input_tokens: Some(input),
        output_tokens: Some(output),
        cache_creation_tokens: Some(input * 2),
        cache_read_tokens: Some(output * 3),
        cost_usd: None,
        duration_ms: None,
        is_sidechain: false,
        content: Some(json!([{ "type": "text", "text": "hi" }])),
        raw,
        search_text: Some("text".into()),
    })
}

fn seed_batch(
    machine_id: Uuid,
    project_path: &str,
    session: &str,
    messages: Vec<Msg>,
) -> IngestBatch {
    let messages: Vec<IngestMessage> = messages.into_iter().map(|m| m.0).collect();
    IngestBatch {
        machine: MachineInfo {
            machine_id,
            hostname: "diff-host".into(),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: project_path.into(),
            name: Some("diffproj".into()),
            storage_type: Some("jsonl".into()),
            session_count: Some(1),
            message_count: Some(i32::try_from(messages.len()).unwrap_or(0)),
            last_modified: None,
            ..Default::default()
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: session.into(),
            project_path: Some(project_path.into()),
            file_path: Some(format!("{project_path}/{session}.jsonl")),
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

// --- the gate ---------------------------------------------------------------

fn as_json<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("serialize stats")
}

/// Reports the first differing path rather than dumping two large blobs, which
/// is the difference between "the port is wrong somewhere" and a fix.
fn assert_same(label: &str, pg: &Value, duck: &Value) {
    if pg == duck {
        return;
    }
    let mut trail = Vec::new();
    first_difference(pg, duck, String::new(), &mut trail);
    panic!(
        "{label}: postgres and duckdb rollups disagree\n  first difference at: {}\n\
         postgres: {}\n  duckdb: {}",
        trail.first().map_or("(root)", String::as_str),
        pg,
        duck
    );
}

fn first_difference(a: &Value, b: &Value, path: String, out: &mut Vec<String>) {
    if !out.is_empty() || a == b {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, av) in x {
                match y.get(k) {
                    Some(bv) => first_difference(av, bv, format!("{path}.{k}"), out),
                    None => out.push(format!("{path}.{k} (missing on duckdb side)")),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (av, bv)) in x.iter().zip(y).enumerate() {
                first_difference(av, bv, format!("{path}[{i}]"), out);
            }
        }
        _ => out.push(format!("{path} ({a} != {b})")),
    }
}

/// Every window shape the endpoints accept, since the scope predicate is the
/// part of the port most able to differ without any single rollup being wrong.
fn windows() -> Vec<(&'static str, Window)> {
    vec![
        ("unbounded/UTC", Window::default()),
        (
            "unbounded/Europe-Rome",
            Window {
                from: None,
                to: None,
                tz: chrono_tz::Europe::Rome,
            },
        ),
        (
            "bounded/UTC",
            Window {
                from: Some("2026-07-20".parse().unwrap()),
                to: Some("2026-07-21".parse().unwrap()),
                tz: chrono_tz::UTC,
            },
        ),
        (
            // The bucket boundary moves with the zone: the 23:40Z message on
            // the 21st is the 22nd in Rome, and the 02:05Z one is still the
            // 22nd. Both implementations must agree on which rows are in.
            "bounded/Europe-Rome",
            Window {
                from: Some("2026-07-21".parse().unwrap()),
                to: Some("2026-07-22".parse().unwrap()),
                tz: chrono_tz::Europe::Rome,
            },
        ),
    ]
}

#[tokio::test]
async fn project_rollups_match_between_postgres_and_duckdb() {
    let f = spawn_and_seed().await;
    let conn = f.mirror.connection().expect("mirror connection");

    for (label, w) in windows() {
        let pg = stats::project(&f.pool, "diffproj".into(), vec![f.project_path.clone()], &w)
            .await
            .expect("postgres project stats");
        let duck = stats_duck::project(&conn, "diffproj".into(), vec![f.project_path.clone()], &w)
            .expect("duckdb project stats");
        assert_same(&format!("project/{label}"), &as_json(&pg), &as_json(&duck));
    }
}

#[tokio::test]
async fn session_rollups_match_between_postgres_and_duckdb() {
    let f = spawn_and_seed().await;
    let conn = f.mirror.connection().expect("mirror connection");

    for (label, w) in windows() {
        let pg = stats::session(&f.pool, f.session_pk, &w)
            .await
            .expect("postgres session stats");
        let duck = stats_duck::session(&conn, f.session_pk, &w).expect("duckdb session stats");
        assert_same(&format!("session/{label}"), &as_json(&pg), &as_json(&duck));
    }

    // And an id that does not exist must be `None` on both, not an error and
    // not a zeroed body.
    assert!(stats::session(&f.pool, -1, &Window::default())
        .await
        .expect("postgres missing session")
        .is_none());
    assert!(stats_duck::session(&conn, -1, &Window::default())
        .expect("duckdb missing session")
        .is_none());
}

/// Global scope covers whatever else the shared test database holds, which
/// makes it the broadest comparison available here — every other test's rows
/// are fixture data this one gets for free.
#[tokio::test]
async fn global_rollups_match_between_postgres_and_duckdb() {
    let f = spawn_and_seed().await;
    let conn = f.mirror.connection().expect("mirror connection");

    for (label, w) in windows() {
        let pg = stats::global(&f.pool, &w).await.expect("postgres global");
        let duck = stats_duck::global(&conn, &w).expect("duckdb global");
        assert_same(&format!("global/{label}"), &as_json(&pg), &as_json(&duck));
    }
}

/// The one divergence design D3 predicts, asserted rather than left to be
/// rediscovered as a bug.
///
/// `usage_row` is computed over the whole archive, so for a logical message
/// whose rows straddle a window edge, the mirror attributes usage to the row
/// that is the global group minimum — which may sit outside the window, while
/// Postgres (deduping *after* windowing) picks the earliest row inside it.
/// Measured on the live archive: 10 of 2,648,869 groups straddle a date
/// boundary, and it only matters when a window edge falls exactly there.
#[tokio::test]
async fn boundary_straddling_group_is_the_documented_d3_divergence() {
    let f = spawn_and_seed().await;

    // One logical message with rows on either side of midnight UTC.
    let machine_id: Uuid =
        sqlx::query_scalar("SELECT machine_id FROM machines WHERE hostname = 'diff-host' LIMIT 1")
            .fetch_one(&f.pool)
            .await
            .expect("machine");
    // Its own session, because the test database persists between runs: reusing
    // a seeded session would accumulate one straddling group per run and the
    // assertion below would read 1000, then 2000, then 3000. Isolation here is
    // what makes the expected numbers exact rather than "some multiple of".
    let project_id: i64 =
        sqlx::query_scalar("SELECT project_id FROM sessions WHERE machine_id = $1 LIMIT 1")
            .bind(machine_id)
            .fetch_one(&f.pool)
            .await
            .expect("project");
    let run = Uuid::new_v4();
    let session_id: i64 = sqlx::query_scalar(
        "INSERT INTO sessions (machine_id, provider, session_id, project_id)
         VALUES ($1, 'claude', $2, $3) RETURNING id",
    )
    .bind(machine_id)
    .bind(format!("straddle-session-{run}"))
    .bind(project_id)
    .fetch_one(&f.pool)
    .await
    .expect("insert straddle session");
    for ts in ["2026-07-23T23:59:30Z", "2026-07-24T00:00:30Z"] {
        sqlx::query(
            r#"INSERT INTO messages
                 (machine_id, session_id, uuid, message_id, provider, role, "timestamp",
                  input_tokens, content, message_key, raw)
               VALUES ($1, $2, $3, $6, 'claude', 'assistant', $4::timestamptz,
                       1000, '[]'::jsonb, $5, '{}'::jsonb)"#,
        )
        .bind(machine_id)
        .bind(session_id)
        .bind(Uuid::new_v4())
        .bind(ts)
        .bind(format!("straddle-{run}-{ts}"))
        .bind(format!("msg_straddle_{run}"))
        .execute(&f.pool)
        .await
        .expect("insert straddling row");
    }
    f.mirror.refresh(&f.pool).await.expect("refresh");
    let conn = f.mirror.connection().expect("connection");

    // A window starting the day AFTER the group's global minimum row.
    let w = Window {
        from: Some("2026-07-24".parse().unwrap()),
        to: Some("2026-07-24".parse().unwrap()),
        tz: chrono_tz::UTC,
    };
    let pg = stats::session(&f.pool, session_id, &w)
        .await
        .expect("pg")
        .expect("session exists");
    let duck = stats_duck::session(&conn, session_id, &w)
        .expect("duck")
        .expect("session exists");

    // Postgres re-deduplicates inside the window, so the 00:00:30 row becomes
    // the usage row and its tokens count. The mirror already decided the
    // 23:59:30 row owns the usage, and that row is outside the window.
    assert_eq!(
        pg.total_input_tokens, 1000,
        "postgres attributes usage to the earliest row *within* the window"
    );
    assert_eq!(
        duck.total_input_tokens, 0,
        "the mirror's usage row for this group sits outside the window — \
         the documented D3 divergence, not a regression"
    );
    assert!(
        pg.total_input_tokens >= duck.total_input_tokens,
        "the divergence is one-directional: the mirror can only under-attribute \
         a straddling group, never double count it"
    );
}
