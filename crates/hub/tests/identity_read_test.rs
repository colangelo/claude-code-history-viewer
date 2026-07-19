//! Integration tests for project identity: `identity:<key>` filter expansion
//! on the read endpoints, the alias round trip, worktree exclusion, and the
//! `/v1/identities` listing.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).
//! Each test uses a fresh random `machine_id` so data is isolated within one
//! shared database; project paths embed the machine id so identity keys and
//! path sets never collide across tests either.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use uuid::Uuid;

/// Per-test root commit derived from the test's fresh machine id: identity
/// keys are global (not machine-scoped), so a fixed root would collide with
/// rows persisted by previous runs against the shared test database.
fn root_hex(machine_id: Uuid) -> String {
    format!("{}00000000", machine_id.simple())
}

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests")
}

struct TestHub {
    base: String,
    token: String,
    machine_id: Uuid,
    #[allow(dead_code)]
    pool: PgPool,
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

    let state = hub::AppState {
        pool: pool.clone(),
        tokens: Arc::new(tokens),
        trusted_identities: Arc::new(Vec::new()),
    };
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
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn get_json(hub: &TestHub, path_and_query: &str) -> Value {
    let resp = client()
        .get(format!("{}{path_and_query}", hub.base))
        .bearer_auth(&hub.token)
        .send()
        .await
        .expect("request");
    assert!(
        resp.status().is_success(),
        "GET {path_and_query} -> {}",
        resp.status()
    );
    resp.json().await.expect("json body")
}

/// A project row with a fingerprint plus one session with one message, at
/// `path`, with the session's first/last message time at `ts`.
fn seeded_batch(
    hub: &TestHub,
    path: &str,
    remote: Option<&str>,
    worktree: bool,
    session: &str,
    ts: &str,
    text: &str,
) -> IngestBatch {
    IngestBatch {
        machine: MachineInfo {
            machine_id: hub.machine_id,
            hostname: format!("host-{}", hub.machine_id),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: path.into(),
            name: Some("proj".into()),
            git_root_commit: Some(root_hex(hub.machine_id)),
            git_remote_url: remote.map(Into::into),
            git_is_worktree: Some(worktree),
            ..Default::default()
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: session.into(),
            project_path: Some(path.into()),
            file_path: Some(format!("{path}/{session}.jsonl")),
            entrypoint: None,
            summary: Some("s".into()),
            message_count: Some(1),
            first_message_time: Some(ts.into()),
            last_message_time: Some(ts.into()),
            last_modified: Some(ts.into()),
            has_tool_use: Some(false),
            has_errors: Some(false),
            storage_type: Some("jsonl".into()),
        }],
        messages: vec![IngestMessage {
            provider: "claude".into(),
            session_id: session.into(),
            message_key: format!("{session}-m1"),
            uuid: Some(format!("{session}-u1")),
            parent_uuid: None,
            seq: 0,
            timestamp: Some(ts.into()),
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
            content: Some(json!([{ "type": "text", "text": text }])),
            raw: json!({ "text": text }),
            search_text: Some(text.into()),
        }],
    }
}

async fn ingest(hub: &TestHub, batch: &IngestBatch) {
    let resp = client()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.token)
        .json(batch)
        .send()
        .await
        .expect("ingest");
    assert_eq!(resp.status(), 200, "ingest failed");
}

fn enc(s: &str) -> String {
    // Minimal query-string escaping for the values used here.
    s.replace('%', "%25")
        .replace('&', "%26")
        .replace('+', "%2B")
        .replace(':', "%3A")
        .replace('/', "%2F")
        .replace('|', "%7C")
        .replace('@', "%40")
}

/// Seed the canonical moved-repo scenario: live NEW path + live-but-different
/// OLD path (no fingerprint — simulating a dead/moved dir that was archived
/// before fingerprinting existed). Returns `(identity_key, old_path, new_path)`.
async fn seed_moved_repo(hub: &TestHub, remote_repo: &str) -> (String, String, String) {
    let m = hub.machine_id;
    let old_path = format!("/tmp/id-{m}/old/foo");
    let new_path = format!("/tmp/id-{m}/new/foo");

    // Old path: archived WITHOUT fingerprint (pre-identity history).
    let mut old = seeded_batch(
        hub,
        &old_path,
        None,
        false,
        &format!("old-sess-{m}"),
        "2026-07-01T12:00:00Z",
        "identity haystack alpha",
    );
    old.projects[0].git_root_commit = None;
    old.projects[0].git_is_worktree = None;
    ingest(hub, &old).await;

    // New path: fingerprinted.
    let new = seeded_batch(
        hub,
        &new_path,
        Some(remote_repo),
        false,
        &format!("new-sess-{m}"),
        "2026-07-10T12:00:00Z",
        "identity haystack beta",
    );
    ingest(hub, &new).await;

    let key = format!(
        "g:{}|{}",
        root_hex(hub.machine_id),
        remote_repo
            .trim_start_matches("git@")
            .replace(".com:", ".com/")
            .trim_end_matches(".git")
    );
    (key, old_path, new_path)
}

#[tokio::test]
async fn projects_listing_exposes_identity_fields() {
    let hub = spawn().await;
    let path = format!("/tmp/id-{}/solo", hub.machine_id);
    ingest(
        &hub,
        &seeded_batch(
            &hub,
            &path,
            Some("git@github.com:acme/solo.git"),
            true,
            &format!("solo-{}", hub.machine_id),
            "2026-07-10T12:00:00Z",
            "solo text",
        ),
    )
    .await;

    let projects = get_json(&hub, "/v1/projects?limit=200").await;
    let row = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["project_path"] == json!(path))
        .expect("seeded project listed");
    assert_eq!(
        row["identity_key"],
        json!(format!(
            "g:{}|github.com/acme/solo",
            root_hex(hub.machine_id)
        ))
    );
    assert_eq!(row["git_worktree"], json!(true));
}

#[tokio::test]
async fn identity_filter_unions_members_and_aliases_on_sessions_and_search() {
    let hub = spawn().await;
    let (key, old_path, new_path) = seed_moved_repo(&hub, "git@github.com:acme/moved.git").await;

    // Identity scope alone: only the fingerprinted new path.
    let sessions = get_json(
        &hub,
        &format!("/v1/sessions?project=identity:{}", enc(&key)),
    )
    .await;
    let paths: Vec<&str> = sessions
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["project_path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&new_path.as_str()));
    assert!(
        !paths.contains(&old_path.as_str()),
        "old path not yet aliased"
    );

    // Alias the dead path in.
    let resp = client()
        .post(format!("{}/v1/identities/aliases", hub.base))
        .bearer_auth(&hub.token)
        .json(&json!({ "project_path": old_path, "identity_key": key }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let alias: Value = resp.json().await.unwrap();
    let alias_id = alias["id"].as_i64().expect("alias id");

    let sessions = get_json(
        &hub,
        &format!("/v1/sessions?project=identity:{}", enc(&key)),
    )
    .await;
    let paths: Vec<String> = sessions
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["project_path"].as_str().unwrap().to_string())
        .collect();
    assert!(paths.contains(&new_path), "member path included");
    assert!(paths.contains(&old_path), "aliased path included");

    // Search unions both paths as one corpus.
    let results = get_json(
        &hub,
        &format!("/v1/search?q=haystack&project=identity:{}", enc(&key)),
    )
    .await;
    let hit_paths: Vec<String> = results["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["project_path"].as_str().unwrap().to_string())
        .collect();
    assert!(hit_paths.contains(&new_path));
    assert!(hit_paths.contains(&old_path));

    // Delete the alias: the split is restored.
    let resp = client()
        .delete(format!("{}/v1/identities/aliases/{alias_id}", hub.base))
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
    let sessions = get_json(
        &hub,
        &format!("/v1/sessions?project=identity:{}", enc(&key)),
    )
    .await;
    let paths: Vec<String> = sessions
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["project_path"].as_str().unwrap().to_string())
        .collect();
    assert!(!paths.contains(&old_path), "unlinked path excluded again");
}

#[tokio::test]
async fn plain_project_filter_is_unchanged() {
    let hub = spawn().await;
    let (_key, old_path, new_path) = seed_moved_repo(&hub, "git@github.com:acme/plain.git").await;

    let sessions = get_json(&hub, &format!("/v1/sessions?project={}", enc(&new_path))).await;
    let arr = sessions.as_array().unwrap();
    assert_eq!(
        arr.len(),
        1,
        "plain path filter matches exactly one project"
    );
    assert_eq!(arr[0]["project_path"], json!(new_path));
    let _ = old_path;
}

#[tokio::test]
async fn unknown_identity_matches_nothing() {
    let hub = spawn().await;
    seed_moved_repo(&hub, "git@github.com:acme/unknown.git").await;
    let sessions = get_json(
        &hub,
        &format!(
            "/v1/sessions?project=identity:{}",
            enc("g:ffffffffffffffffffffffffffffffffffffffff")
        ),
    )
    .await;
    assert_eq!(sessions.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn worktree_members_are_excludable() {
    let hub = spawn().await;
    let m = hub.machine_id;
    let main_path = format!("/tmp/id-{m}/wt/main");
    let wt_path = format!("/tmp/id-{m}/wt/feature");
    let remote = "git@github.com:acme/wt.git";
    ingest(
        &hub,
        &seeded_batch(
            &hub,
            &main_path,
            Some(remote),
            false,
            &format!("wt-main-{m}"),
            "2026-07-10T12:00:00Z",
            "wt main text",
        ),
    )
    .await;
    ingest(
        &hub,
        &seeded_batch(
            &hub,
            &wt_path,
            Some(remote),
            true,
            &format!("wt-feat-{m}"),
            "2026-07-11T12:00:00Z",
            "wt feature text",
        ),
    )
    .await;
    let key = format!("g:{}|github.com/acme/wt", root_hex(hub.machine_id));

    let with = get_json(
        &hub,
        &format!("/v1/sessions?project=identity:{}", enc(&key)),
    )
    .await;
    assert_eq!(
        with.as_array().unwrap().len(),
        2,
        "worktrees included by default"
    );

    let without = get_json(
        &hub,
        &format!(
            "/v1/sessions?project=identity:{}&include_worktrees=false",
            enc(&key)
        ),
    )
    .await;
    let paths: Vec<&str> = without
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["project_path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec![main_path.as_str()], "worktree member excluded");
}

#[tokio::test]
async fn fork_with_same_root_is_never_grouped() {
    let hub = spawn().await;
    let m = hub.machine_id;
    let ours = format!("/tmp/id-{m}/fork/ours");
    let theirs = format!("/tmp/id-{m}/fork/theirs");
    ingest(
        &hub,
        &seeded_batch(
            &hub,
            &ours,
            Some("git@github.com:colangelo/fk.git"),
            false,
            &format!("fk-ours-{m}"),
            "2026-07-10T12:00:00Z",
            "fork ours",
        ),
    )
    .await;
    ingest(
        &hub,
        &seeded_batch(
            &hub,
            &theirs,
            Some("git@github.com:upstream/fk.git"),
            false,
            &format!("fk-theirs-{m}"),
            "2026-07-10T13:00:00Z",
            "fork theirs",
        ),
    )
    .await;

    let ours_key = format!("g:{}|github.com/colangelo/fk", root_hex(hub.machine_id));
    let sessions = get_json(
        &hub,
        &format!("/v1/sessions?project=identity:{}", enc(&ours_key)),
    )
    .await;
    let paths: Vec<&str> = sessions
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["project_path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec![ours.as_str()], "fork stays a distinct identity");

    // ...but surfaces as a related_identity suggestion.
    let identities = get_json(&hub, "/v1/identities").await;
    let ours_id = identities
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["identity_key"] == json!(ours_key))
        .expect("our identity listed");
    let related: Vec<&str> = ours_id["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["kind"] == json!("related_identity"))
        .map(|s| s["identity_key"].as_str().unwrap())
        .collect();
    assert!(
        related
            .contains(&format!("g:{}|github.com/upstream/fk", root_hex(hub.machine_id)).as_str()),
        "fork suggested as related"
    );
}

#[tokio::test]
async fn identities_listing_carries_members_aliases_and_orphan_suggestions() {
    let hub = spawn().await;
    let (key, old_path, new_path) = seed_moved_repo(&hub, "git@github.com:acme/listing.git").await;

    let identities = get_json(&hub, "/v1/identities").await;
    let id = identities
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["identity_key"] == json!(key))
        .expect("identity listed");
    assert_eq!(id["display_name"], json!("foo"));
    let member_paths: Vec<&str> = id["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|mem| mem["project_path"].as_str().unwrap())
        .collect();
    assert_eq!(member_paths, vec![new_path.as_str()]);
    // The dead old path (same basename, no fingerprint) is suggested.
    let orphan_suggested = id["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["kind"] == json!("orphan_path") && s["project_path"] == json!(old_path));
    assert!(orphan_suggested, "orphan path suggested by basename");

    // After aliasing, the path moves from suggestion to alias.
    let resp = client()
        .post(format!("{}/v1/identities/aliases", hub.base))
        .bearer_auth(&hub.token)
        .json(&json!({ "project_path": old_path, "identity_key": key }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let identities = get_json(&hub, "/v1/identities").await;
    let id = identities
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["identity_key"] == json!(key))
        .unwrap();
    let alias_paths: Vec<&str> = id["aliases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["project_path"].as_str().unwrap())
        .collect();
    assert_eq!(alias_paths, vec![old_path.as_str()]);
    let created_by = id["aliases"][0]["created_by"].as_str().unwrap();
    assert!(
        created_by.starts_with("machine:"),
        "audit principal recorded, got {created_by}"
    );
    let orphan_still = id["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["kind"] == json!("orphan_path") && s["project_path"] == json!(old_path));
    assert!(!orphan_still, "aliased path no longer suggested");
}

#[tokio::test]
async fn alias_with_unknown_key_is_rejected() {
    let hub = spawn().await;
    let resp = client()
        .post(format!("{}/v1/identities/aliases", hub.base))
        .bearer_auth(&hub.token)
        .json(&json!({
            "project_path": "/tmp/nowhere",
            "identity_key": "g:0000000000000000000000000000000000000000"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn journal_reads_accept_identity_scope() {
    let hub = spawn().await;
    let (key, old_path, new_path) = seed_moved_repo(&hub, "git@github.com:acme/journal.git").await;

    // Post one journal entry per path (sessions seeded at 12:00 UTC on
    // 2026-07-01 / 2026-07-10 → those are their logical dates).
    for (path, date, sess_text) in [
        (&old_path, "2026-07-01", "journal old chapter"),
        (&new_path, "2026-07-10", "journal new chapter"),
    ] {
        // Surrogate ids come from the sessions listing for the path.
        let sessions = get_json(&hub, &format!("/v1/sessions?project={}", enc(path))).await;
        let ids: Vec<i64> = sessions
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_i64().unwrap())
            .collect();
        assert!(!ids.is_empty());
        let resp = client()
            .post(format!("{}/v1/journal/entries", hub.base))
            .bearer_auth(&hub.token)
            .json(&json!({
                "entry_date": date,
                "project_path": path,
                "status": "entry",
                "headline": sess_text,
                "summary": format!("Summary: {sess_text}."),
                "topics": ["one", "two", "three"],
                "open_questions": [],
                "session_ids": ids,
                "model": "test-model",
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "journal entry create");
    }

    // Without the alias only the new path's entry is in identity scope.
    let entries = get_json(
        &hub,
        &format!("/v1/journal/entries?project=identity:{}", enc(&key)),
    )
    .await;
    assert_eq!(entries.as_array().unwrap().len(), 1);

    // Alias the old path → the full timeline reads as one stream.
    let resp = client()
        .post(format!("{}/v1/identities/aliases", hub.base))
        .bearer_auth(&hub.token)
        .json(&json!({ "project_path": old_path, "identity_key": key }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    let entries = get_json(
        &hub,
        &format!("/v1/journal/entries?project=identity:{}", enc(&key)),
    )
    .await;
    let dates: Vec<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["entry_date"].as_str().unwrap())
        .collect();
    assert_eq!(
        dates,
        vec!["2026-07-10", "2026-07-01"],
        "both paths, newest first"
    );

    // Journal search block honors the scope too.
    let results = get_json(
        &hub,
        &format!(
            "/v1/search?q=chapter&scope=journal&project=identity:{}",
            enc(&key)
        ),
    )
    .await;
    let journal_paths: Vec<&str> = results["journal"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["project_path"].as_str().unwrap())
        .collect();
    assert!(journal_paths.contains(&old_path.as_str()));
    assert!(journal_paths.contains(&new_path.as_str()));
}
