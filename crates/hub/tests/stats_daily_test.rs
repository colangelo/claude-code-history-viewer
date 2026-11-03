//! `GET /v1/stats/daily` — change `stats-read-api`, task 2.1: one test per D2
//! column over a populated mirror, plus the parameter rules (D3 condition 2).
//! The client-scope half (lab sessions only, in observe and enforce) is in
//! `client_scope_routes_test.rs`, beside the R2-E differential test it extends.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use hub::config::MirrorConfig;
use hub::mirror::Mirror;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use uuid::Uuid;

/// sha256 hex: the hub keeps client-reader tokens by hash only.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    Sha256::digest(s.as_bytes())
        .iter()
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests")
}

struct Hub {
    base: String,
    /// A client reader: these tests check the arithmetic over every session
    /// on a hub without client scope (the lab filter has its own tests in
    /// `client_scope_routes_test.rs`).
    token: String,
    /// A machine token, a lab principal: 503 here (security T3).
    lab_token: String,
    pool: PgPool,
    mirror: Arc<Mirror>,
    machine: Uuid,
}

async fn spawn() -> Hub {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");
    let machine = Uuid::new_v4();
    sqlx::query("INSERT INTO machines (machine_id, hostname) VALUES ($1, $2)")
        .bind(machine)
        .bind(format!("daily-{}", machine.simple()))
        .execute(&pool)
        .await
        .expect("insert machine");
    let lab_token = format!("tok-{machine}");
    let token = format!("creader-{machine}");
    let dir = std::env::temp_dir().join(format!("cchv-daily-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mirror = Arc::new(
        Mirror::open_or_create(&MirrorConfig {
            path: Some(dir.join("stats.duckdb")),
            ..MirrorConfig::default()
        })
        .expect("open mirror"),
    );
    let state = hub::AppState::new(
        pool.clone(),
        HashMap::from([(lab_token.clone(), machine)]),
        Vec::new(),
    )
    .with_client_readers(HashMap::from([(sha256_hex(&token), machine)]))
    .with_mirror(mirror.clone());
    let app = hub::router(state, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Hub {
        base: format!("http://{addr}"),
        token,
        lab_token,
        pool,
        mirror,
        machine,
    }
}

impl Hub {
    /// A project (its own identity key) with one session; returns the session row id.
    async fn session(&self, project_name: &str) -> i64 {
        let tag = Uuid::new_v4().simple().to_string();
        let project_id: i64 = sqlx::query_scalar(
            "INSERT INTO projects (machine_id, provider, project_path, name, identity_key)
             VALUES ($1, 'claude', $2, $3, $4) RETURNING id",
        )
        .bind(self.machine)
        .bind(format!("/tmp/daily-{tag}"))
        .bind(project_name)
        .bind(format!("key-{tag}"))
        .fetch_one(&self.pool)
        .await
        .expect("insert project");
        sqlx::query_scalar(
            "INSERT INTO sessions (machine_id, provider, session_id, project_id)
             VALUES ($1, 'claude', $2, $3) RETURNING id",
        )
        .bind(self.machine)
        .bind(tag)
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .expect("insert session")
    }

    #[allow(clippy::too_many_arguments)]
    async fn row(
        &self,
        session: i64,
        ts: &str,
        role: Option<&str>,
        content: Option<Value>,
        raw: Value,
        tokens: (Option<i64>, Option<i64>),
        cost: Option<f64>,
    ) {
        sqlx::query(
            r#"INSERT INTO messages
                 (machine_id, session_id, uuid, provider, role, "timestamp", content,
                  message_key, raw, input_tokens, output_tokens, cost_usd)
               VALUES ($1, $2, $3, 'claude', $4, $5::timestamptz, $6, $3::text, $7, $8, $9, $10)"#,
        )
        .bind(self.machine)
        .bind(session)
        .bind(Uuid::new_v4())
        .bind(role)
        .bind(ts)
        .bind(content)
        .bind(raw)
        .bind(tokens.0)
        .bind(tokens.1)
        .bind(cost)
        .execute(&self.pool)
        .await
        .expect("insert message");
    }

    async fn get(&self, query: &str) -> (u16, Value) {
        self.mirror.refresh(&self.pool).await.expect("refresh");
        let r = reqwest::Client::new()
            .get(format!("{}/v1/stats/daily{query}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }
}

/// The row of `project` on `day`, from a daily answer.
fn row<'a>(body: &'a Value, day: &str, project: &str) -> &'a Value {
    body["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["day"] == day && r["project"] == project)
        .unwrap_or_else(|| panic!("no row for {project} on {day}: {body}"))
}

#[tokio::test]
async fn every_d2_column_over_one_worked_day() {
    let hub = spawn().await;
    let name = format!("dailyproj-{}", Uuid::new_v4().simple());
    let s = hub.session(&name).await;
    let user = || json!({"type": "user"});
    // 2026-10-05, Europe/Rome is UTC+2: 08:00Z is 10:00 local.
    hub.row(
        s,
        "2026-10-05T08:00:00Z",
        Some("user"),
        Some(json!("first prompt")),
        user(),
        (None, None),
        None,
    )
    .await;
    // +60 s
    hub.row(
        s,
        "2026-10-05T08:01:00Z",
        Some("assistant"),
        Some(json!([{"type": "text", "text": "a"}])),
        json!({"type": "assistant"}),
        (Some(100), Some(20)),
        Some(0.5),
    )
    .await;
    // +60 s, a tool result only: not a prompt
    hub.row(
        s,
        "2026-10-05T08:02:00Z",
        Some("user"),
        Some(json!([{"type": "tool_result", "tool_use_id": "x", "content": "ok"}])),
        user(),
        (None, None),
        None,
    )
    .await;
    // +1800 s: capped at 300
    hub.row(
        s,
        "2026-10-05T08:32:00Z",
        Some("user"),
        Some(json!("second prompt")),
        user(),
        (None, None),
        None,
    )
    .await;
    // +10 s: compaction boundary, then its summary prompt (+5 s): not a turn
    hub.row(
        s,
        "2026-10-05T08:32:10Z",
        Some("system"),
        Some(json!("Conversation compacted")),
        json!({"type": "system", "subtype": "compact_boundary"}),
        (None, None),
        None,
    )
    .await;
    hub.row(
        s,
        "2026-10-05T08:32:15Z",
        Some("user"),
        Some(json!("This session is being continued...")),
        user(),
        (None, None),
        None,
    )
    .await;
    // +45 s: an assistant row with no content (bookkeeping): not a message
    hub.row(
        s,
        "2026-10-05T08:33:00Z",
        Some("assistant"),
        None,
        json!({"type": "assistant"}),
        (Some(50), Some(5)),
        Some(0.25),
    )
    .await;

    let (status, body) = hub.get("?from=2026-10-05&to=2026-10-05").await;
    assert_eq!(status, 200, "{body}");
    let r = row(&body, "2026-10-05", &name);
    assert_eq!(r["space"], "ac");
    assert_eq!(r["sessions"], 1);
    assert_eq!(r["messages"], 6, "rows with content");
    assert_eq!(r["turns"], 2, "3 prompts minus 1 compaction");
    assert_eq!(r["compactions"], 1);
    assert_eq!(r["tokens_in"], 150);
    assert_eq!(r["tokens_out"], 25);
    assert_eq!(
        r["active_seconds"],
        60 + 60 + 300 + 10 + 5 + 45,
        "gaps capped at 300 s"
    );
    assert_eq!(r["ac_messages"], Value::Null);
    assert_eq!(r["cost_usd"], 0.75);
    assert_eq!(body["tz"], "Europe/Rome", "the default zone");
}

#[tokio::test]
async fn a_day_without_reported_cost_is_null_not_zero() {
    let hub = spawn().await;
    let name = format!("nocost-{}", Uuid::new_v4().simple());
    let s = hub.session(&name).await;
    hub.row(
        s,
        "2026-10-06T09:00:00Z",
        Some("user"),
        Some(json!("hi")),
        json!({}),
        (None, None),
        None,
    )
    .await;
    hub.row(
        s,
        "2026-10-06T09:00:30Z",
        Some("assistant"),
        Some(json!([{"type": "text", "text": "a"}])),
        json!({}),
        (Some(10), Some(2)),
        None,
    )
    .await;
    let (status, body) = hub.get("?from=2026-10-06&to=2026-10-06").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(row(&body, "2026-10-06", &name)["cost_usd"], Value::Null);
}

#[tokio::test]
async fn days_are_local_to_tz() {
    let hub = spawn().await;
    let name = format!("tzproj-{}", Uuid::new_v4().simple());
    let s = hub.session(&name).await;
    // 23:30Z on the 7th is 01:30 on the 8th in Rome.
    hub.row(
        s,
        "2026-10-07T23:30:00Z",
        Some("user"),
        Some(json!("late")),
        json!({}),
        (None, None),
        None,
    )
    .await;
    let (_, rome) = hub.get("?from=2026-10-07&to=2026-10-08").await;
    row(&rome, "2026-10-08", &name);
    let (_, utc) = hub.get("?from=2026-10-07&to=2026-10-08&tz=UTC").await;
    row(&utc, "2026-10-07", &name);
}

#[tokio::test]
async fn parameters_are_from_to_tz_only_within_31_days() {
    let hub = spawn().await;
    for (query, why) in [
        ("?from=2026-09-01&to=2026-10-02", "32 days"),
        ("?from=2026-10-02&to=2026-10-01", "from after to"),
        (
            "?from=2026-10-01&to=2026-10-02&project=x",
            "an unknown parameter",
        ),
        (
            "?from=2026-10-01&to=2026-10-02&session=1",
            "an unknown parameter",
        ),
        (
            "?from=2026-10-01&to=2026-10-02&tz=Not/A_Zone",
            "an unknown zone",
        ),
    ] {
        let (status, body) = hub.get(query).await;
        assert_eq!(status, 400, "{why}: {query} -> {body}");
    }
    let (status, body) = hub.get("?from=2026-09-01&to=2026-10-01").await;
    assert_eq!(status, 200, "31 days is allowed: {body}");
    let (status, _) = hub.get("").await;
    assert_eq!(status, 200, "defaults: the last 31 days");
}

#[tokio::test]
async fn needs_a_read_token() {
    let hub = spawn().await;
    let r = reqwest::Client::new()
        .get(format!("{}/v1/stats/daily", hub.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
}

/// Security T3: a lab principal on a hub with no client scope gets 503, not
/// every session.
#[tokio::test]
async fn a_lab_reader_gets_503_without_client_scope() {
    let hub = spawn().await;
    hub.mirror.refresh(&hub.pool).await.expect("refresh");
    let since = (chrono::Utc::now() - chrono::Duration::hours(1))
        .to_rfc3339()
        .replace('+', "%2B");
    let url = if "daily" == "daily" {
        format!("{}/v1/stats/daily", hub.base)
    } else {
        format!("{}/v1/stats/tool-calls?prefix=x&since={since}", hub.base)
    };
    let r = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&hub.lab_token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 503);
    let r = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap();
    assert_ne!(
        r.status().as_u16(),
        503,
        "the control: a client reader is served"
    );
}

/// A day's project name does not depend on the requested window (infra,
/// 2026-11-03): an identity's name is chosen from all of its visible project
/// rows, not only from those with a session inside the window.
#[tokio::test]
async fn a_days_project_name_does_not_depend_on_the_window() {
    let hub = spawn().await;
    let key = format!("key-{}", Uuid::new_v4().simple());
    let tag = Uuid::new_v4().simple().to_string();
    let (alpha, beta) = (format!("alpha-{tag}"), format!("beta-{tag}"));
    let mut sessions = Vec::new();
    for (i, name) in [&alpha, &beta, &beta].into_iter().enumerate() {
        let project_id: i64 = sqlx::query_scalar(
            "INSERT INTO projects (machine_id, provider, project_path, name, identity_key)
             VALUES ($1, 'claude', $2, $3, $4) RETURNING id",
        )
        .bind(hub.machine)
        .bind(format!("/tmp/daily-name-{tag}-{i}"))
        .bind(name)
        .bind(&key)
        .fetch_one(&hub.pool)
        .await
        .expect("insert project");
        let s: i64 = sqlx::query_scalar(
            "INSERT INTO sessions (machine_id, provider, session_id, project_id)
             VALUES ($1, 'claude', $2, $3) RETURNING id",
        )
        .bind(hub.machine)
        .bind(format!("{tag}-{i}"))
        .bind(project_id)
        .fetch_one(&hub.pool)
        .await
        .expect("insert session");
        sessions.push(s);
    }
    // alpha's session on 10-05 only; the two beta rows on 10-06 and 10-07.
    for (s, ts) in sessions.iter().zip([
        "2026-10-05T10:00:00Z",
        "2026-10-06T10:00:00Z",
        "2026-10-07T10:00:00Z",
    ]) {
        hub.row(
            *s,
            ts,
            Some("user"),
            Some(json!([{"type": "text", "text": "hi"}])),
            json!({"type": "user"}),
            (None, None),
            None,
        )
        .await;
    }
    let names_on_0505 = |body: &Value| -> Vec<String> {
        body["rows"]
            .as_array()
            .expect("rows")
            .iter()
            .filter(|r| {
                r["day"] == "2026-10-05" && r["project"].as_str().is_some_and(|p| p.ends_with(&tag))
            })
            .map(|r| r["project"].as_str().unwrap().to_string())
            .collect()
    };
    let (s1, narrow) = hub.get("?from=2026-10-05&to=2026-10-05&tz=UTC").await;
    let (s2, wide) = hub.get("?from=2026-10-05&to=2026-10-07&tz=UTC").await;
    assert_eq!((s1, s2), (200, 200));
    assert_eq!(names_on_0505(&narrow), vec![beta.clone()], "{narrow}");
    assert_eq!(names_on_0505(&narrow), names_on_0505(&wide));
}
