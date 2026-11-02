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
    dir: TempDir,
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
        dir: TempDir(dir),
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

/// R2-B: a journal entry the distiller wrote on the protected route makes its
/// sessions one client family, at the POST.
#[tokio::test]
async fn a_protected_entry_makes_its_sessions_client() {
    let hub = spawn().await;
    let s = sid();
    let project = format!("/w/pe-{s}");
    let mut row = session(&s);
    row["project_path"] = json!(project);
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        row,
        vec![msg(
            &s,
            text("lab, but term-protected"),
            json!({"cwd": "/w/lab"}),
        )],
    )
    .await;
    assert_eq!(
        state_of(&hub, hub.machine, &s).await.0.as_deref(),
        Some("none")
    );
    let pk: i64 = sqlx::query_scalar("SELECT id FROM sessions WHERE session_id = $1")
        .bind(&s)
        .fetch_one(&hub.pool)
        .await
        .unwrap();
    let r = reqwest::Client::new()
        .post(format!("{}/v1/journal/entries", hub.base))
        .bearer_auth(&hub.token)
        .json(&json!({"entry_date": "2026-10-30", "project_path": project, "status": "entry",
                      "headline": "h", "summary": "s", "topics": ["a", "b", "c"], "open_questions": [],
                      "session_ids": [pk], "model": "claude-x", "route": "protected"}))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    assert_eq!(status, 200, "{}", r.text().await.unwrap());
    assert_eq!(
        state_of(&hub, hub.machine, &s).await.0.as_deref(),
        Some("family:protected-entry")
    );
}

/// R2-I: once the hub has enforced, a config that says observe (a stale
/// render from the launcher's cached fallback) does not turn enforcement off.
#[tokio::test]
async fn the_latch_outlives_an_observe_config() {
    let hub = spawn().await; // observe mode, latch cleared
    let scope_cfg = |enforce: bool| {
        let dir = &hub.dir.0;
        toml::from_str::<hub::HubConfig>(&format!(
            "database_url = \"unused\"\n[client_scope]\nrouting_file = \"{}\"\nfloor_file = \"{}\"\nenforce = {enforce}\n",
            dir.join("routing.toml").display(),
            dir.join("floor.json").display()
        ))
        .unwrap()
    };
    let observe = hub::AppState::from_config(hub.pool.clone(), &scope_cfg(false)).unwrap();
    let s = observe.client_scope.clone().unwrap();
    s.refresh(&hub.pool).await.unwrap();
    assert!(!s.enforcing(), "observe before any enforcing start");

    let enforce = hub::AppState::from_config(hub.pool.clone(), &scope_cfg(true)).unwrap();
    enforce
        .client_scope
        .clone()
        .unwrap()
        .refresh(&hub.pool)
        .await
        .unwrap();

    let stale = hub::AppState::from_config(hub.pool.clone(), &scope_cfg(false)).unwrap();
    let s = stale.client_scope.clone().unwrap();
    s.refresh(&hub.pool).await.unwrap();
    assert!(s.enforcing(), "the latch holds");
    assert!(s.latch_overrides_config(), "and says so");
}

/// Security K1: the latch is in the database, so a config with no
/// `[client_scope]` section must not lift it. Lab readers then see nothing.
#[tokio::test]
async fn a_latched_hub_without_the_section_shows_lab_readers_nothing() {
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
    sqlx::query("UPDATE client_scope_state SET enforced_at = now()")
        .execute(&hub.pool)
        .await
        .unwrap();

    let reader = format!("tok-r-{}", Uuid::new_v4());
    let config: hub::HubConfig = toml::from_str(&format!(
        "database_url = \"unused\"\n[[readers]]\ntoken = \"{reader}\"\nmachine_id = \"{}\"\nlabel = \"r\"\n",
        Uuid::new_v4()
    ))
    .unwrap();
    let state = hub::AppState::from_config(hub.pool.clone(), &config)
        .unwrap()
        .with_orphan_latch()
        .await
        .unwrap();
    let scope = state
        .client_scope
        .clone()
        .expect("the latch turns client scope on");
    scope.refresh(&hub.pool).await.unwrap();
    assert!(scope.enforcing() && scope.snapshot().generation().is_none());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = hub::router(state, None);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let body = reqwest::Client::new()
        .get(format!("{base}/v1/sessions?limit=500"))
        .bearer_auth(&reader)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "[]", "a lab reader sees no session");
    let health = reqwest::get(format!("{base}/v1/healthz/client-scope"))
        .await
        .unwrap();
    assert_eq!(health.status().as_u16(), 503);
    sqlx::query("UPDATE client_scope_state SET enforced_at = NULL")
        .execute(&hub.pool)
        .await
        .unwrap();
}

/// Security O2: a batch ingested while the policy is unusable cannot be
/// classified, so the session it touches loses its coverage. When the same
/// routing file returns, the session stays unscanned until a sweep reads it.
#[tokio::test]
async fn ingest_during_a_policy_outage_leaves_the_session_unscanned() {
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
    assert_eq!(
        state_of(&hub, hub.machine, &s).await,
        (Some("none".into()), true)
    );

    let routing = hub.dir.0.join("routing.toml");
    let away = hub.dir.0.join("routing.away");
    std::fs::rename(&routing, &away).unwrap();
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        session(&s),
        vec![msg(&s, text("now clientco."), json!({"cwd": "/w/lab"}))],
    )
    .await;
    std::fs::rename(&away, &routing).unwrap();
    // The next batch reloads the identical file: same generation as before.
    let other = sid();
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        session(&other),
        vec![msg(&other, text("lab"), json!({"cwd": "/w/lab"}))],
    )
    .await;
    assert_eq!(
        state_of(&hub, hub.machine, &other).await,
        (Some("none".into()), true),
        "policy back"
    );
    assert_eq!(
        state_of(&hub, hub.machine, &s).await,
        (Some("none".into()), false),
        "unscanned: not lab until a sweep reads it"
    );

    let hub::client_policy::Load::Ready(policy) =
        hub::client_policy::load_paths(&routing, &hub.dir.0.join("floor.json"))
    else {
        panic!("policy")
    };
    let pk: i64 = sqlx::query_scalar("SELECT id FROM sessions WHERE session_id = $1")
        .bind(&s)
        .fetch_one(&hub.pool)
        .await
        .unwrap();
    hub::client_sweep::sweep(
        &hub.pool,
        &policy,
        &hub::client_sweep::SweepOptions {
            dry_run: false,
            batch: 10,
            only_sessions: Some(vec![pk]),
            rescan: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        state_of(&hub, hub.machine, &s).await,
        (Some("text_marker".into()), true)
    );
}

/// Security E3: the pending list tells the distiller the hub's class of each
/// group, so a group the hub holds client (or has not scanned) never goes to
/// the general model. `lab` only when every session is lab under the current
/// generation.
#[tokio::test]
async fn pending_groups_carry_the_hubs_class() {
    let hub = spawn().await;
    // The newest day the route lists (its day starts at 04:00 UTC), and paths
    // that sort first: the route pages newest day and highest path first, at
    // most 200 groups, and the shared test database keeps every run's groups.
    let day_date =
        (chrono::Utc::now() - chrono::Duration::hours(4)).date_naive() - chrono::Duration::days(1);
    let day = format!("{day_date}T12:00:00Z");
    let mk = |tag: &str| {
        let s = sid();
        let mut row = session(&s);
        row["project_path"] = json!(format!("/~e3-{tag}-{s}"));
        (s, row)
    };
    let at = |s: &str, text: &str| {
        let mut m = msg(
            s,
            json!([{"type": "text", "text": text}]),
            json!({"cwd": "/w/lab"}),
        );
        m["timestamp"] = json!(day);
        m
    };
    let (lab, lab_row) = mk("lab");
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        lab_row.clone(),
        vec![at(&lab, "plain")],
    )
    .await;
    let (cli, cli_row) = mk("client");
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        cli_row.clone(),
        vec![at(&cli, "see clientco.")],
    )
    .await;
    let (uns, uns_row) = mk("unscanned");
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        uns_row.clone(),
        vec![at(&uns, "plain")],
    )
    .await;
    sqlx::query("UPDATE sessions SET client_gen = NULL WHERE session_id = $1")
        .bind(&uns)
        .execute(&hub.pool)
        .await
        .unwrap();

    let groups: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{}/v1/journal/pending?from={day_date}&limit=200",
            hub.base
        ))
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let class_of = |row: &Value| {
        let path = row["project_path"].as_str().unwrap();
        groups
            .iter()
            .find(|g| g["project_path"] == path)
            .map(|g| g["client_scope"].as_str().unwrap_or("<absent>").to_string())
    };
    assert_eq!(class_of(&lab_row).as_deref(), Some("lab"));
    assert_eq!(class_of(&cli_row).as_deref(), Some("client"));
    assert_eq!(class_of(&uns_row).as_deref(), Some("unscanned"));
}

async fn post_batch(hub: &Hub, projects: Value, sessions: Value, messages: Value) {
    let r = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.token)
        .json(&json!({
            "machine": {"machine_id": hub.machine, "hostname": format!("h-{}", &hub.machine.simple().to_string()[..8]), "os": "macos"},
            "projects": projects,
            "sessions": sessions,
            "messages": messages,
        }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "ingest {}", r.status());
}

/// E1: a batch that only updates a project row (here its git remote arrives
/// later) raises that project's sessions to `path`, and their families with
/// them; a lab remote and another project's session stay lab.
#[tokio::test]
async fn a_project_only_update_raises_its_sessions_and_family() {
    let hub = spawn().await;
    let project = format!("/w/lab-{}", Uuid::new_v4());
    let parent = Uuid::new_v4().to_string();
    let child = sid();
    let other = sid();
    let lab_msg = |s: &str| msg(s, text("lab work"), json!({"cwd": "/w/lab"}));
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        json!({"provider": "claude", "session_id": parent, "project_path": project}),
        vec![lab_msg(&parent)],
    )
    .await;
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        json!({"provider": "claude", "session_id": child, "project_path": "/w/other",
               "file_path": format!("/x/{parent}/subagents/agent-1.jsonl")}),
        vec![lab_msg(&child)],
    )
    .await;
    ingest(
        &hub,
        hub.machine,
        &hub.token,
        json!({"provider": "claude", "session_id": other, "project_path": "/w/other"}),
        vec![lab_msg(&other)],
    )
    .await;

    let remote = |url: &str| json!([{"provider": "claude", "project_path": project, "name": "p", "git_remote_url": url}]);
    post_batch(
        &hub,
        remote("https://gitea.example/ac/lab.git"),
        json!([]),
        json!([]),
    )
    .await;
    assert_eq!(
        state_of(&hub, hub.machine, &parent).await.0.as_deref(),
        Some("none"),
        "a lab remote stays lab"
    );

    post_batch(
        &hub,
        remote("https://gitea.example/clientco/repo.git"),
        json!([]),
        json!([]),
    )
    .await;
    assert_eq!(
        state_of(&hub, hub.machine, &parent).await.0.as_deref(),
        Some("path")
    );
    assert_eq!(
        state_of(&hub, hub.machine, &child).await.0.as_deref(),
        Some("family:path")
    );
    assert_eq!(
        state_of(&hub, hub.machine, &other).await.0.as_deref(),
        Some("none")
    );
}

/// E1: `search_text` is accepted from the client, so it is classified too.
#[tokio::test]
async fn a_marker_only_in_search_text_makes_the_session_client() {
    let hub = spawn().await;
    let s = sid();
    let mut m = msg(&s, text("lab work"), json!({"cwd": "/w/lab"}));
    m["search_text"] = json!("lab work /w/clientco");
    ingest(&hub, hub.machine, &hub.token, session(&s), vec![m]).await;
    assert_eq!(
        state_of(&hub, hub.machine, &s).await,
        (Some("text_marker".into()), true)
    );
}
