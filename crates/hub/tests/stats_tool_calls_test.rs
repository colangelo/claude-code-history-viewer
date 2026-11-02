//! `GET /v1/stats/tool-calls` — change `stats-read-api`, task 3.1 (design D4):
//! counts per tool and per host since T, a last-segment match, the 31-day
//! bound. The client-scope half is in `client_scope_routes_test.rs`.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

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

struct Hub {
    base: String,
    token: String,
    pool: PgPool,
    /// Run-unique, so the shared test database's earlier runs never match.
    tag: String,
}

async fn spawn() -> Hub {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");
    let machine = Uuid::new_v4();
    let token = format!("tok-{machine}");
    let state = hub::AppState::new(pool.clone(), HashMap::from([(token.clone(), machine)]), Vec::new());
    let app = hub::router(state, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Hub {
        base: format!("http://{addr}"),
        token,
        pool,
        tag: Uuid::new_v4().simple().to_string()[..10].to_string(),
    }
}

impl Hub {
    /// A machine named `host` with one session; returns `(machine, session)`.
    async fn host(&self, host: &str) -> (Uuid, i64) {
        let machine = Uuid::new_v4();
        sqlx::query("INSERT INTO machines (machine_id, hostname) VALUES ($1, $2)")
            .bind(machine)
            .bind(host)
            .execute(&self.pool)
            .await
            .unwrap();
        let project: i64 = sqlx::query_scalar(
            "INSERT INTO projects (machine_id, provider, project_path, name)
             VALUES ($1, 'claude', $2, 'p') RETURNING id",
        )
        .bind(machine)
        .bind(format!("/tmp/tc-{}", Uuid::new_v4().simple()))
        .fetch_one(&self.pool)
        .await
        .unwrap();
        let session: i64 = sqlx::query_scalar(
            "INSERT INTO sessions (machine_id, provider, session_id, project_id)
             VALUES ($1, 'claude', $2, $3) RETURNING id",
        )
        .bind(machine)
        .bind(Uuid::new_v4().to_string())
        .bind(project)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        (machine, session)
    }

    /// One message at `ts` carrying one tool use named `tool`.
    async fn call(&self, at: (Uuid, i64), ts: &str, tool: &str) {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO messages
                 (machine_id, session_id, uuid, provider, role, "timestamp", content, message_key, raw)
               VALUES ($1, $2, $3, 'claude', 'assistant', $4::timestamptz, '[]'::jsonb, $3::text, '{}'::jsonb)
               RETURNING id"#,
        )
        .bind(at.0)
        .bind(at.1)
        .bind(Uuid::new_v4())
        .bind(ts)
        .fetch_one(&self.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO message_tool_uses (message_ref, session_id, seq, tool_name) VALUES ($1, $2, 0, $3)",
        )
        .bind(id)
        .bind(at.1)
        .bind(tool)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn get(&self, query: &str) -> (u16, Value) {
        let r = reqwest::Client::new()
            .get(format!("{}/v1/stats/tool-calls{query}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }
}

fn ago(hours: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::hours(hours)).to_rfc3339()
}

fn enc(s: &str) -> String {
    s.replace('+', "%2B").replace(':', "%3A")
}

#[tokio::test]
async fn counts_per_tool_and_host_since_t_by_last_segment() {
    let hub = spawn().await;
    let t = &hub.tag;
    let tool = format!("search_{t}");
    let (a, b) = (format!("hosta-{t}"), format!("hostb-{t}"));
    let on_a = hub.host(&a).await;
    let on_b = hub.host(&b).await;

    hub.call(on_a, &ago(2), &format!("mcp__exa__{tool}")).await;
    hub.call(on_a, &ago(3), &format!("mcp__exa__{tool}")).await;
    hub.call(on_b, &ago(4), &format!("mcp__exa__{tool}")).await;
    hub.call(on_b, &ago(5), &format!("mcp__other__{tool}")).await;
    hub.call(on_b, &ago(6), &tool).await; // a full name matches too
    // Not matches: a longer last segment, another tool, and one before T.
    hub.call(on_a, &ago(2), &format!("mcp__exa__{tool}_v2")).await;
    hub.call(on_a, &ago(2), &format!("mcp__exa__{tool}x")).await;
    hub.call(on_a, &ago(50), &format!("mcp__exa__{tool}")).await;

    let (status, body) = hub.get(&format!("?prefix={tool}&since={}", enc(&ago(24)))).await;
    assert_eq!(status, 200, "{body}");
    let by_tool: Vec<(String, Value, i64)> = body["by_tool"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["tool"].as_str().unwrap().to_string(), r["server"].clone(), r["count"].as_i64().unwrap()))
        .collect();
    assert_eq!(
        by_tool,
        vec![
            (format!("mcp__exa__{tool}"), json!("exa"), 3),
            (format!("mcp__other__{tool}"), json!("other"), 1),
            (tool.clone(), Value::Null, 1),
        ],
        "by count, then name"
    );
    let by_host: Vec<(String, i64)> = body["by_host"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["host"].as_str().unwrap().to_string(), r["count"].as_i64().unwrap()))
        .collect();
    assert_eq!(by_host, vec![(b.clone(), 3), (a.clone(), 2)]);
    assert!(body["last_ingest"].is_string(), "{body}");
    assert!(body["since"].is_string(), "{body}");
}

#[tokio::test]
async fn parameters_are_prefix_and_since_only_within_31_days() {
    let hub = spawn().await;
    let ok_since = enc(&ago(24 * 30));
    for (query, why) in [
        (format!("?prefix=x&since={}", enc(&ago(24 * 32))), "32 days back"),
        (format!("?since={ok_since}"), "no prefix"),
        (format!("?prefix=&since={ok_since}"), "an empty prefix"),
        ("?prefix=x".to_string(), "no since"),
        ("?prefix=x&since=yesterday".to_string(), "since not RFC 3339"),
        (format!("?prefix=x&since={ok_since}&project=y"), "an unknown parameter"),
    ] {
        let (status, body) = hub.get(&query).await;
        assert_eq!(status, 400, "{why}: {query} -> {body}");
    }
    let (status, body) = hub.get(&format!("?prefix=x&since={ok_since}")).await;
    assert_eq!(status, 200, "30 days back is allowed: {body}");
}
