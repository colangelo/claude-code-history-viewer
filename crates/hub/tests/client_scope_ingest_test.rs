//! Client scope at ingest (openspec `hub-client-read-scope`, R2-A, R2-B, R2-C).
//!
//! Requires `TEST_DATABASE_URL`; run with `--test-threads=1` (the policy and
//! latch tables are one row each, and every test resets them).

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL")
}

const ROUTING: &str = r#"
[protected]
model = "claude-x"
path_markers = ["/clientco", "-users-ac--sync-dev--clientco"]
content_terms = ["clientword"]
"#;

struct Hub {
    base: String,
    token: String,
    machine: Uuid,
    pool: PgPool,
    _dir: TempDir,
}

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    Sha256::digest(b).iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

/// A hub as `run()` builds it, with `[client_scope]` on a fresh routing file.
async fn spawn() -> Hub {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&test_db_url())
        .await
        .unwrap();
    hub::MIGRATOR.run(&pool).await.unwrap();
    sqlx::query("DELETE FROM client_scope_policy")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE client_scope_state SET generation = NULL, enforced_at = NULL")
        .execute(&pool)
        .await
        .unwrap();

    let dir = std::env::temp_dir().join(format!("cchv-cs-ingest-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let routing = dir.join("routing.toml");
    let floor = dir.join("floor.json");
    std::fs::write(&routing, ROUTING).unwrap();
    std::fs::write(
        &floor,
        json!({"sha256": hex(ROUTING.as_bytes()), "plain_path_markers": 1,
               "encoded_path_markers": 1, "content_terms": 1})
        .to_string(),
    )
    .unwrap();

    let token = format!("tok-{}", Uuid::new_v4());
    let machine = Uuid::new_v4();
    let config: hub::HubConfig = toml::from_str(&format!(
        "database_url = \"unused\"\n\
         [[tokens]]\ntoken = \"{token}\"\nmachine_id = \"{machine}\"\nlabel = \"m\"\n\
         [client_scope]\nrouting_file = \"{}\"\nfloor_file = \"{}\"\n",
        routing.display(),
        floor.display()
    ))
    .unwrap();
    let state = hub::AppState::from_config(pool.clone(), &config).unwrap();
    state
        .client_scope
        .as_ref()
        .expect("client scope configured")
        .refresh(&state.pool)
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = hub::router(state, None);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Hub {
        base: format!("http://{addr}"),
        token,
        machine,
        pool,
        _dir: TempDir(dir),
    }
}

fn msg(session: &str, content: Value, raw: Value) -> Value {
    json!({"provider": "claude", "session_id": session, "message_key": Uuid::new_v4().to_string(),
           "seq": 0, "timestamp": "2026-10-30T12:00:00Z", "type": "user", "role": "user",
           "content": content, "raw": raw})
}

fn text(t: &str) -> Value {
    json!([{"type": "text", "text": t}])
}

async fn ingest(hub: &Hub, machine: Uuid, token: &str, session: Value, messages: Vec<Value>) {
    let project = session["project_path"]
        .as_str()
        .unwrap_or("/w/lab")
        .to_string();
    let r = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(token)
        .json(&json!({
            "machine": {"machine_id": machine, "hostname": format!("h-{}", &machine.simple().to_string()[..8]), "os": "macos"},
            "projects": [{"provider": "claude", "project_path": project, "name": "p"}],
            "sessions": [session],
            "messages": messages,
        }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "ingest {}", r.status());
}

fn session(id: &str) -> Value {
    json!({"provider": "claude", "session_id": id, "project_path": "/w/lab"})
}

/// (`client_class`, whether `client_gen` is the current generation)
async fn state_of(hub: &Hub, machine: Uuid, session_id: &str) -> (Option<String>, bool) {
    let row: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT s.client_class, s.client_gen, st.generation
         FROM sessions s, client_scope_state st
         WHERE s.machine_id = $1 AND s.session_id = $2",
    )
    .bind(machine)
    .bind(session_id)
    .fetch_one(&hub.pool)
    .await
    .unwrap();
    (row.0, row.1.is_some() && row.1 == row.2)
}

fn sid() -> String {
    format!("cs-{}", Uuid::new_v4())
}

#[tokio::test]
async fn a_new_lab_session_is_scanned_and_lab() {
    let hub = spawn().await;
    let s = sid();
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        session(&s),
        vec![msg(&s, text("lab work"), json!({"cwd": "/w/lab"}))],
    )
    .await;
    assert_eq!(
        state_of(&hub, hub.machine, &s).await,
        (Some("none".into()), true)
    );
}

#[tokio::test]
async fn a_marker_anywhere_served_makes_the_session_client() {
    let hub = spawn().await;
    let big = format!("{} then clientco.", "a".repeat(70_000));
    let cases: Vec<(&str, Value, Value, Value)> = vec![
        (
            "tool_result",
            session(&sid()),
            json!([{"type": "tool_result", "content": [{"type": "text", "text": "x /w/clientco y"}]}]),
            json!({"cwd": "/w/lab"}),
        ),
        (
            "text_marker",
            session(&sid()),
            text(&big),
            json!({"cwd": "/w/lab"}),
        ),
        (
            "cwd",
            session(&sid()),
            text("hello"),
            json!({"cwd": "/w/clientco"}),
        ),
        (
            "raw",
            session(&sid()),
            text("hello"),
            json!({"cwd": "/w/lab", "toolUseResult": {"stdout": "built clientco."}}),
        ),
    ];
    for (want, sess, content, raw) in cases {
        let s = sess["session_id"].as_str().unwrap().to_string();
        ingest(
            &hub,
            hub.machine,
            &hub.token,
            sess,
            vec![msg(&s, content, raw)],
        )
        .await;
        assert_eq!(
            state_of(&hub, hub.machine, &s).await,
            (Some(want.into()), true),
            "{want}"
        );
    }
}

#[tokio::test]
async fn a_metadata_only_update_is_classified() {
    let hub = spawn().await;
    let s = sid();
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        session(&s),
        vec![msg(&s, text("lab"), json!({"cwd": "/w/lab"}))],
    )
    .await;
    let mut row = session(&s);
    row["summary"] = json!("fixed the /w/clientco build");
    ingest(&hub, hub.machine, &hub.token, row, vec![]).await;
    assert_eq!(
        state_of(&hub, hub.machine, &s).await,
        (Some("metadata".into()), true)
    );
}

#[tokio::test]
async fn the_class_only_rises() {
    let hub = spawn().await;
    let s = sid();
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        session(&s),
        vec![msg(&s, text("now clientco."), json!({"cwd": "/w/lab"}))],
    )
    .await;
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        session(&s),
        vec![msg(&s, text("harmless"), json!({"cwd": "/w/lab"}))],
    )
    .await;
    assert_eq!(
        state_of(&hub, hub.machine, &s).await,
        (Some("text_marker".into()), true)
    );
}

/// A harmless batch never stamps a session the hub has not fully scanned
/// under the current generation (R2-C): it stays unscanned for the sweep.
#[tokio::test]
async fn a_harmless_batch_does_not_stamp_an_unscanned_session() {
    let hub = spawn().await;
    let s = sid();
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        session(&s),
        vec![msg(&s, text("lab"), json!({"cwd": "/w/lab"}))],
    )
    .await;
    // As if the session predates client scope, or an older generation scanned it.
    sqlx::query("UPDATE sessions SET client_class = NULL, client_gen = NULL WHERE session_id = $1")
        .bind(&s)
        .execute(&hub.pool)
        .await
        .unwrap();
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        session(&s),
        vec![msg(&s, text("harmless"), json!({"cwd": "/w/lab"}))],
    )
    .await;
    assert_eq!(
        state_of(&hub, hub.machine, &s).await,
        (None, false),
        "still unscanned"
    );
    // A marker still raises it, scanned or not.
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        session(&s),
        vec![msg(&s, text("clientco."), json!({"cwd": "/w/lab"}))],
    )
    .await;
    assert_eq!(
        state_of(&hub, hub.machine, &s).await,
        (Some("text_marker".into()), false)
    );
}

/// R2-B: copies of one provider session id on two machines, and a subagent
/// transcript whose path names its parent, are one family.
#[tokio::test]
async fn a_family_is_protected_together() {
    let hub = spawn().await;
    // Parent and subagent child on one machine: the child's path names the parent.
    let parent = Uuid::new_v4().to_string();
    let child = Uuid::new_v4().to_string();
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        session(&parent),
        vec![msg(&parent, text("lab"), json!({"cwd": "/w/lab"}))],
    )
    .await;
    let mut child_row = session(&child);
    child_row["file_path"] = json!(format!(
        "/Users/ac/.claude/projects/-w-lab/{parent}/subagents/agent-1.jsonl"
    ));
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        child_row,
        vec![msg(
            &child,
            text("built clientco."),
            json!({"cwd": "/w/lab"}),
        )],
    )
    .await;
    assert_eq!(
        state_of(&hub, hub.machine, &child).await.0.as_deref(),
        Some("text_marker")
    );
    assert_eq!(
        state_of(&hub, hub.machine, &parent).await.0.as_deref(),
        Some("family:text_marker"),
        "a child hit protects the parent"
    );

    // A later child of a client parent inherits at ingest.
    let child2 = Uuid::new_v4().to_string();
    let mut row2 = session(&child2);
    row2["file_path"] = json!(format!("/x/{parent}/subagents/agent-2.jsonl"));
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        row2,
        vec![msg(&child2, text("lab"), json!({"cwd": "/w/lab"}))],
    )
    .await;
    assert_eq!(
        state_of(&hub, hub.machine, &child2).await.0.as_deref(),
        Some("family:text_marker")
    );
}
