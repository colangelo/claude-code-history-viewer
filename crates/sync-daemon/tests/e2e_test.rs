//! End-to-end: a fixture `~/.claude` session → the real daemon sync → the real
//! hub → Postgres → `GET /v1/search`. Exercises the whole MVP in one process.
//!
//! Requires `TEST_DATABASE_URL` (or `DATABASE_URL`). `$HOME` is process-global,
//! so this is `#[serial]` and the suite runs with `--test-threads=1`.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use sync_daemon::checkpoint::Checkpoint;
use sync_daemon::client::ReqwestHubClient;
use sync_daemon::identity::Identity;
use sync_daemon::sync;
use tempfile::TempDir;
use tokio::net::TcpListener;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

fn write_claude_fixture(home: &std::path::Path) {
    let dir = home.join(".claude/projects/-Users-test-proj");
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join("sess-e2e.jsonl")).unwrap();
    writeln!(
        f,
        r#"{{"uuid":"e1","sessionId":"sess-e2e","timestamp":"2026-01-01T00:00:00Z","type":"user","cwd":"/Users/test/proj","message":{{"role":"user","content":"the peregrine falcon dives fast"}}}}"#
    )
    .unwrap();
    writeln!(
        f,
        r#"{{"uuid":"e2","parentUuid":"e1","sessionId":"sess-e2e","timestamp":"2026-01-01T00:01:00Z","type":"assistant","message":{{"role":"assistant","model":"claude-x","content":[{{"type":"text","text":"indeed it does"}}]}}}}"#
    )
    .unwrap();
}

#[tokio::test]
#[serial]
async fn daemon_to_hub_to_search() {
    // --- Postgres + hub --------------------------------------------------
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect db");
    hub::MIGRATOR.run(&pool).await.expect("migrate");

    // --- fixture machine + identity --------------------------------------
    let home = TempDir::new().unwrap();
    std::env::set_var("HOME", home.path());
    let state_dir = home.path().join("sync-state");
    let identity = Identity::load_or_create(&state_dir).unwrap();
    write_claude_fixture(home.path());

    // --- spawn the real hub, trusting this machine's token ---------------
    let token = format!("e2e-{}", identity.machine_id);
    let mut tokens = HashMap::new();
    tokens.insert(token.clone(), identity.machine_id);
    let state = hub::AppState {
        pool: pool.clone(),
        tokens: Arc::new(tokens),
        trusted_identities: Arc::new(Vec::new()),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    tokio::spawn(async move {
        axum::serve(listener, hub::router(state, None)).await.unwrap();
    });

    // --- run the daemon against the real hub -----------------------------
    let client = ReqwestHubClient::new(&base, &token);
    let mut cp = Checkpoint::load(&state_dir);
    let stats = sync::run_once(&client, &identity, &mut cp, 500, &[]).await;
    assert!(
        stats.sessions_synced >= 1,
        "daemon synced the fixture session"
    );
    assert!(stats.messages_delivered >= 2);

    // --- search the archive ----------------------------------------------
    let resp = reqwest::Client::new()
        .get(format!("{base}/v1/search"))
        .query(&[("q", "falcon"), ("machine", identity.hostname.as_str())])
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    // The shared test DB accumulates across runs, so assert our backfilled
    // message is present (not an exact count): the pipeline reached search.
    let hit = results
        .iter()
        .find(|r| r["session_id"] == "sess-e2e")
        .expect("the backfilled session is searchable");
    assert!(hit["snippet"].as_str().unwrap().contains("falcon"));
    assert_eq!(hit["machine_hostname"], identity.hostname);
}
