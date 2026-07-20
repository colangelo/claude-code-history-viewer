//! Integration tests for the read API (search + browse).
//!
//! Read endpoints span every machine in the archive, so each test uses a unique
//! hostname and scopes its queries with `machine=<hostname>` to stay isolated
//! within the shared test database. Requires `TEST_DATABASE_URL`/`DATABASE_URL`.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use tokio::net::TcpListener;
use uuid::Uuid;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

struct TestHub {
    base: String,
    token: String,
    machine_id: Uuid,
    hostname: String,
}

async fn spawn() -> TestHub {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect");
    hub::MIGRATOR.run(&pool).await.expect("migrate");

    let machine_id = Uuid::new_v4();
    let hostname = format!("host-{}", &machine_id.simple().to_string()[..12]);
    let token = format!("tok-{machine_id}");
    let mut tokens = HashMap::new();
    tokens.insert(token.clone(), machine_id);

    let state = hub::AppState::new(pool, tokens, Vec::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, hub::router(state, None))
            .await
            .unwrap();
    });

    TestHub {
        base: format!("http://{addr}"),
        token,
        machine_id,
        hostname,
    }
}

fn proj(path: &str, name: &str) -> IngestProject {
    IngestProject {
        provider: "claude".into(),
        project_path: path.into(),
        name: Some(name.into()),
        storage_type: Some("jsonl".into()),
        session_count: None,
        message_count: None,
        last_modified: None,
        ..Default::default()
    }
}

fn sess(session: &str, project_path: &str) -> IngestSession {
    IngestSession {
        provider: "claude".into(),
        session_id: session.into(),
        project_path: Some(project_path.into()),
        file_path: None,
        entrypoint: None,
        summary: Some(format!("summary of {session}")),
        message_count: None,
        first_message_time: None,
        last_message_time: None,
        last_modified: None,
        has_tool_use: Some(false),
        has_errors: Some(false),
        storage_type: Some("jsonl".into()),
    }
}

fn msg(session: &str, key: &str, seq: i32, ts: &str, text: &str) -> IngestMessage {
    IngestMessage {
        provider: "claude".into(),
        session_id: session.into(),
        message_key: key.into(),
        uuid: Some(key.into()),
        parent_uuid: None,
        seq,
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
    }
}

async fn ingest(hub: &TestHub, batch: &IngestBatch) {
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.token)
        .json(batch)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "ingest setup failed");
}

fn batch(
    hub: &TestHub,
    projects: Vec<IngestProject>,
    sessions: Vec<IngestSession>,
    messages: Vec<IngestMessage>,
) -> IngestBatch {
    IngestBatch {
        machine: MachineInfo {
            machine_id: hub.machine_id,
            hostname: hub.hostname.clone(),
            os: Some("macos".into()),
        },
        projects,
        sessions,
        messages,
    }
}

async fn get(
    hub: &TestHub,
    path: &str,
    query: &[(&str, &str)],
    token: Option<&str>,
) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .get(format!("{}{}", hub.base, path))
        .query(query);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap()
}

#[tokio::test]
async fn healthz_reports_ok_unauthenticated() {
    let hub = spawn().await;
    // /v1/healthz is unauthenticated and reflects database connectivity.
    let resp = get(&hub, "/v1/healthz", &[], None).await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["db"], "up");
}

#[tokio::test]
async fn search_returns_ranked_matches_with_context() {
    let hub = spawn().await;
    let b = batch(
        &hub,
        vec![proj("/tmp/a", "alpha")],
        vec![sess("s1", "/tmp/a")],
        vec![
            msg("s1", "k1", 0, "2026-01-01T00:00:00Z", "the quick brown fox"),
            msg("s1", "k2", 1, "2026-01-01T00:01:00Z", "a slow green turtle"),
        ],
    );
    ingest(&hub, &b).await;

    let resp = get(
        &hub,
        "/v1/search",
        &[("q", "fox"), ("machine", &hub.hostname)],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["session_id"], "s1");
    assert_eq!(results[0]["project_name"], "alpha");
    assert_eq!(results[0]["machine_hostname"], hub.hostname);
    assert!(results[0]["rank"].as_f64().unwrap() > 0.0);
    assert!(results[0]["snippet"].as_str().unwrap().contains("fox"));
}

#[tokio::test]
async fn search_filters_narrow_results() {
    let hub = spawn().await;
    let b = batch(
        &hub,
        vec![proj("/tmp/a", "alpha"), proj("/tmp/b", "beta")],
        vec![sess("sa", "/tmp/a"), sess("sb", "/tmp/b")],
        vec![
            msg("sa", "ka", 0, "2026-01-01T00:00:00Z", "shared needle alpha"),
            msg("sb", "kb", 0, "2026-01-01T00:00:00Z", "shared needle beta"),
        ],
    );
    ingest(&hub, &b).await;

    // Without a project filter: both match "needle".
    let all = get(
        &hub,
        "/v1/search",
        &[("q", "needle"), ("machine", &hub.hostname)],
        Some(&hub.token),
    )
    .await;
    let all: Value = all.json().await.unwrap();
    assert_eq!(all["results"].as_array().unwrap().len(), 2);

    // With project=alpha: only the alpha hit.
    let filtered = get(
        &hub,
        "/v1/search",
        &[
            ("q", "needle"),
            ("machine", &hub.hostname),
            ("project", "alpha"),
        ],
        Some(&hub.token),
    )
    .await;
    let filtered: Value = filtered.json().await.unwrap();
    let r = filtered["results"].as_array().unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0]["project_name"], "alpha");
}

#[tokio::test]
async fn search_no_matches_is_empty_200() {
    let hub = spawn().await;
    let b = batch(
        &hub,
        vec![proj("/tmp/a", "alpha")],
        vec![sess("s1", "/tmp/a")],
        vec![msg("s1", "k1", 0, "2026-01-01T00:00:00Z", "ordinary words")],
    );
    ingest(&hub, &b).await;

    let resp = get(
        &hub,
        "/v1/search",
        &[("q", "zzzznevermatches"), ("machine", &hub.hostname)],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["results"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn projects_list_carries_provenance_and_aggregates() {
    let hub = spawn().await;
    let b = batch(
        &hub,
        vec![proj("/tmp/a", "alpha")],
        vec![sess("s1", "/tmp/a")],
        vec![
            msg("s1", "k1", 0, "2026-01-01T00:00:00Z", "one"),
            msg("s1", "k2", 1, "2026-01-01T00:01:00Z", "two"),
        ],
    );
    ingest(&hub, &b).await;

    let resp = get(
        &hub,
        "/v1/projects",
        &[("machine", &hub.hostname)],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let projects: Value = resp.json().await.unwrap();
    let arr = projects.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "alpha");
    assert_eq!(arr[0]["machine_hostname"], hub.hostname);
    assert_eq!(arr[0]["session_count"], 1);
    assert_eq!(arr[0]["message_count"], 2);
}

#[tokio::test]
async fn session_messages_returned_in_order() {
    let hub = spawn().await;
    // Insert messages out of seq order; the endpoint must return them by seq.
    let b = batch(
        &hub,
        vec![proj("/tmp/a", "alpha")],
        vec![sess("s1", "/tmp/a")],
        vec![
            msg("s1", "k2", 2, "2026-01-01T00:02:00Z", "third"),
            msg("s1", "k0", 0, "2026-01-01T00:00:00Z", "first"),
            msg("s1", "k1", 1, "2026-01-01T00:01:00Z", "second"),
        ],
    );
    ingest(&hub, &b).await;

    // Resolve the surrogate session id via the sessions list.
    let sessions = get(
        &hub,
        "/v1/sessions",
        &[("machine", &hub.hostname)],
        Some(&hub.token),
    )
    .await;
    let sessions: Value = sessions.json().await.unwrap();
    let sid = sessions[0]["id"].as_i64().unwrap();

    let msgs = get(
        &hub,
        &format!("/v1/sessions/{sid}/messages"),
        &[],
        Some(&hub.token),
    )
    .await;
    assert_eq!(msgs.status(), 200);
    let msgs: Value = msgs.json().await.unwrap();
    let seqs: Vec<i64> = msgs
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, vec![0, 1, 2], "messages must be ordered by seq");
}

#[tokio::test]
async fn session_messages_accepts_session_uuid() {
    let hub = spawn().await;
    // A realistic provider session id (UUID), not the surrogate PK.
    let suid = Uuid::new_v4().to_string();
    let b = batch(
        &hub,
        vec![proj("/tmp/a", "alpha")],
        vec![sess(&suid, "/tmp/a")],
        vec![
            msg(&suid, "k0", 0, "2026-01-01T00:00:00Z", "first"),
            msg(&suid, "k1", 1, "2026-01-01T00:01:00Z", "second"),
        ],
    );
    ingest(&hub, &b).await;

    let msgs = get(
        &hub,
        &format!("/v1/sessions/{suid}/messages"),
        &[],
        Some(&hub.token),
    )
    .await;
    assert_eq!(msgs.status(), 200, "session UUID in the path must resolve");
    let msgs: Value = msgs.json().await.unwrap();
    assert_eq!(msgs.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn session_messages_multi_file_sessions_are_chronological() {
    let hub = spawn().await;
    // Subagent transcript files carry the parent session id, so one hub session
    // can aggregate several files, each with its own seq numbering from 0.
    // Ordering must be chronological, not seq-major (which interleaves files).
    let suid = Uuid::new_v4().to_string();
    let b = batch(
        &hub,
        vec![proj("/tmp/a", "alpha")],
        vec![sess(&suid, "/tmp/a")],
        vec![
            // main transcript
            msg(&suid, "main-0", 0, "2026-01-01T00:00:00Z", "m0"),
            msg(&suid, "main-1", 1, "2026-01-01T00:01:00Z", "m1"),
            msg(&suid, "main-2", 2, "2026-01-01T00:04:00Z", "m2"),
            // subagent transcript, seq restarts at 0, runs between m1 and m2
            msg(&suid, "agent-0", 0, "2026-01-01T00:02:00Z", "a0"),
            msg(&suid, "agent-1", 1, "2026-01-01T00:03:00Z", "a1"),
        ],
    );
    ingest(&hub, &b).await;

    let msgs = get(
        &hub,
        &format!("/v1/sessions/{suid}/messages"),
        &[],
        Some(&hub.token),
    )
    .await;
    let msgs: Value = msgs.json().await.unwrap();
    let keys: Vec<&str> = msgs
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["message_key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        vec!["main-0", "main-1", "agent-0", "agent-1", "main-2"],
        "messages must be in chronological order across files"
    );
}

#[tokio::test]
async fn session_messages_reports_total_count_so_truncation_is_detectable() {
    let hub = spawn().await;
    let suid = Uuid::new_v4().to_string();
    let messages: Vec<IngestMessage> = (0..3)
        .map(|i| {
            msg(
                &suid,
                &format!("k{i}"),
                i,
                &format!("2026-01-01T00:0{i}:00Z"),
                "x",
            )
        })
        .collect();
    let b = batch(
        &hub,
        vec![proj("/tmp/a", "alpha")],
        vec![sess(&suid, "/tmp/a")],
        messages,
    );
    ingest(&hub, &b).await;

    let resp = get(
        &hub,
        &format!("/v1/sessions/{suid}/messages"),
        &[("limit", "2")],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let total = resp
        .headers()
        .get("x-total-count")
        .expect("X-Total-Count header must be present")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(
        total, "3",
        "header carries the session's total message count"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body.as_array().unwrap().len(),
        2,
        "body is still the limited page"
    );
}

#[tokio::test]
async fn session_messages_unknown_session_uuid_is_404() {
    let hub = spawn().await;
    let resp = get(
        &hub,
        &format!("/v1/sessions/{}/messages", Uuid::new_v4()),
        &[],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn session_messages_ambiguous_session_uuid_is_400_with_candidates() {
    // Two machines (two hub handles on the shared DB) archive the same
    // provider session id; resolving it by UUID must refuse with candidates.
    let hub_a = spawn().await;
    let hub_b = spawn().await;
    let suid = Uuid::new_v4().to_string();
    for hub in [&hub_a, &hub_b] {
        let b = batch(
            hub,
            vec![proj("/tmp/a", "alpha")],
            vec![sess(&suid, "/tmp/a")],
            vec![msg(&suid, "k0", 0, "2026-01-01T00:00:00Z", "hello")],
        );
        ingest(hub, &b).await;
    }

    let resp = get(
        &hub_a,
        &format!("/v1/sessions/{suid}/messages"),
        &[],
        Some(&hub_a.token),
    )
    .await;
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    let err = body["error"].as_str().unwrap();
    assert!(
        err.contains("ambiguous"),
        "error should say ambiguous: {err}"
    );
}

#[tokio::test]
async fn unauthenticated_read_is_401() {
    let hub = spawn().await;
    let resp = get(&hub, "/v1/projects", &[], None).await;
    assert_eq!(resp.status(), 401);
    let resp = get(&hub, "/v1/search", &[("q", "x")], Some("bad-token")).await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn paging_is_stable() {
    let hub = spawn().await;
    // Five sessions; page through them two at a time and assert no dup/drop.
    let projects = vec![proj("/tmp/a", "alpha")];
    let sessions: Vec<IngestSession> = (0..5).map(|i| sess(&format!("s{i}"), "/tmp/a")).collect();
    let messages: Vec<IngestMessage> = (0..5)
        .map(|i| {
            msg(
                &format!("s{i}"),
                &format!("k{i}"),
                0,
                &format!("2026-01-0{}T00:00:00Z", i + 1),
                "x",
            )
        })
        .collect();
    ingest(&hub, &batch(&hub, projects, sessions, messages)).await;

    let mut seen = std::collections::HashSet::new();
    for offset in ["0", "2", "4"] {
        let page = get(
            &hub,
            "/v1/sessions",
            &[
                ("machine", &hub.hostname),
                ("limit", "2"),
                ("offset", offset),
            ],
            Some(&hub.token),
        )
        .await;
        let page: Value = page.json().await.unwrap();
        for s in page.as_array().unwrap() {
            let id = s["id"].as_i64().unwrap();
            assert!(
                seen.insert(id),
                "session {id} appeared on more than one page"
            );
        }
    }
    assert_eq!(seen.len(), 5, "all five sessions paged exactly once");
}

// ---------------------------------------------------------------------------
// issue #19 — FTS prefix matching for plain queries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_prefix_matches_word_stems() {
    let hub = spawn().await;
    ingest(
        &hub,
        &batch(
            &hub,
            vec![proj("/w/stems", "stems")],
            vec![sess("s-stems", "/w/stems")],
            vec![msg(
                "s-stems",
                "k-stems-1",
                0,
                "2026-01-01T00:00:00Z",
                "the distiller runs nightly",
            )],
        ),
    )
    .await;

    // Whole-lexeme websearch alone would return nothing for `distill` —
    // the prefix variant must find `distiller`.
    let resp = get(
        &hub,
        "/v1/search",
        &[("q", "distill"), ("machine", &hub.hostname)],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "prefix query must match `distiller`");
    assert_eq!(results[0]["session_id"], "s-stems");

    // Advanced websearch syntax disables the prefix variant: negation keeps
    // exact whole-lexeme semantics, so `distill` alone matches nothing.
    let resp = get(
        &hub,
        "/v1/search",
        &[("q", "distill -nightly"), ("machine", &hub.hostname)],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["results"].as_array().unwrap().len(),
        0,
        "advanced syntax must keep exact websearch semantics"
    );
}

#[tokio::test]
async fn journal_search_prefix_matches_word_stems() {
    let hub = spawn().await;
    // Seed a journal entry directly (text_search is GENERATED from
    // search_text); the POST path requires full session provenance which is
    // irrelevant to FTS behavior.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&test_db_url())
        .await
        .unwrap();
    let project = format!("/w/journal-{}", hub.hostname);
    sqlx::query(
        "INSERT INTO journal_entries
             (entry_date, project_path, status, headline, summary, model, search_text)
         VALUES ('2026-01-02', $1, 'entry', 'Distiller hardening',
                 'Hardened the distiller against DNS flakes.', 'test-model',
                 'Distiller hardening. Hardened the distiller against DNS flakes.')",
    )
    .bind(&project)
    .execute(&pool)
    .await
    .unwrap();

    let resp = get(
        &hub,
        "/v1/search",
        &[
            ("q", "distill"),
            ("project", &project),
            ("scope", "journal"),
        ],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let journal = body["journal"].as_array().unwrap();
    assert_eq!(
        journal.len(),
        1,
        "journal prefix query must match `distiller`"
    );
    assert_eq!(journal[0]["project_path"], Value::String(project));
}

// ---------------------------------------------------------------------------
// issue #20 — search hits carry their browse-order position
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_hits_carry_browse_order_position() {
    let hub = spawn().await;
    ingest(
        &hub,
        &batch(
            &hub,
            vec![proj("/w/pos", "pos")],
            vec![sess("s-pos", "/w/pos")],
            vec![
                msg(
                    "s-pos",
                    "k-pos-0",
                    0,
                    "2026-01-01T00:00:00Z",
                    "alpha filler",
                ),
                msg("s-pos", "k-pos-1", 1, "2026-01-01T00:01:00Z", "beta filler"),
                msg(
                    "s-pos",
                    "k-pos-2",
                    2,
                    "2026-01-01T00:02:00Z",
                    "needle-zzq here",
                ),
            ],
        ),
    )
    .await;

    let resp = get(
        &hub,
        "/v1/search",
        &[("q", "needle-zzq"), ("machine", &hub.hostname)],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    // Third message in browse ordering (timestamp ASC) → 0-based position 2.
    assert_eq!(results[0]["position"], 2);

    // Sanity: the position indexes into the browse messages listing.
    let session_pk = results[0]["session_pk"].as_i64().unwrap();
    let resp = get(
        &hub,
        &format!("/v1/sessions/{session_pk}/messages"),
        &[],
        Some(&hub.token),
    )
    .await;
    let messages: Value = resp.json().await.unwrap();
    assert_eq!(
        messages.as_array().unwrap()[2]["message_key"],
        results[0]["message_key"]
    );
}
