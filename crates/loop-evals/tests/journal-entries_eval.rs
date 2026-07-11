//! T2 acceptance evals for the hub **journal-entries** surface (issue #12).
//!
//! Contract under test (all NEW routes — none exist on the unmodified crate, so
//! every request below currently fails at runtime with 404/405, which is the
//! correct pre-implementation RED):
//!
//! * `GET  /v1/journal/pending`  (read-auth) — data-derived work list of
//!   `(entry_date, project_path)` groups, top-level JSON array, newest-first,
//!   honoring `from` (date lower bound) + `limit`. Each group:
//!   `{ entry_date: "YYYY-MM-DD", project_path, session_ids: [<surrogate int>] }`.
//! * `POST /v1/journal/entries`  (machine-token, same model as `/v1/ingest`) —
//!   validated upsert by `(entry_date, project_path)`. Body:
//!   `{ entry_date, project_path, status: "entry"|"skip", headline, summary,
//!      topics: [str], open_questions: [str], session_ids: [int], model }`.
//!   2xx on success; 400/422 (never a partial write) on invalid payloads.
//! * `GET  /v1/journal/entries` (read-auth) — browse `entry`-status rows,
//!   `project` filter (matches `project_path`), newest-first, full content
//!   incl. `session_ids`. Skip rows never surface.
//! * `GET  /v1/search?scope=all|messages|journal` — adds a separate additive
//!   `journal` array alongside the unchanged message `results`.
//!
//! ## Shared-database isolation
//!
//! These evals run against a shared `TEST_DATABASE_URL` that is never
//! truncated, and `/v1/journal/pending` + `/v1/journal/entries` + `/v1/search`
//! all span every machine/project in the archive. Isolation strategy:
//!
//! * **Unique keys.** Every test uses a fresh random `project_path` and
//!   `hostname`, so no other test's (or prior run's) data can collide with the
//!   `(entry_date, project_path)` groups asserted here. All presence/absence
//!   assertions filter the global response down to this test's `project_path`.
//! * **Recent, closed logical dates + `from` bound.** Groups are stamped a few
//!   days before "now" (definitely closed under `day_start_hour = 4 UTC`) and
//!   the `pending` queries pass `from = <this test's earliest date>`, which
//!   excludes the mountain of fixed-2026-01/03 data other suites ingest.
//! * **Unique FTS terms.** Search tests seed a per-test random lexeme so only
//!   this test's message + entry can match.
//!
//! Seeding is done exclusively through `POST /v1/ingest` (the public surface),
//! per the RUNBOOK; no raw SQL. Dirty-detection (AC6) needs no backdating:
//! posting an entry stamps `generated_at = now`, then ingesting another session
//! for the group bumps `sessions.updated_at` past it.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use chrono::{Duration, NaiveDate, Utc};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::sync::Arc;
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
    hostname: String,
    project: String,
}

async fn spawn() -> TestHub {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");

    let machine_id = Uuid::new_v4();
    let tag = machine_id.simple().to_string();
    let token = format!("tok-{machine_id}");
    let mut tokens = HashMap::new();
    tokens.insert(token.clone(), machine_id);

    let state = hub::AppState {
        pool,
        tokens: Arc::new(tokens),
        trusted_identities: Arc::new(Vec::new()),
    };
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
        hostname: format!("host-{}", &tag[..12]),
        project: format!("/tmp/journal-{tag}"),
    }
}

/// A closed calendar day `n` days before now (always closed under any
/// `day_start_hour` when `n >= 2`).
fn days_ago(n: i64) -> NaiveDate {
    (Utc::now() - Duration::days(n)).date_naive()
}

/// The current *open* logical date under the default `day_start_hour = 4 UTC`:
/// `pending` MUST exclude it.
fn open_logical_date() -> NaiveDate {
    (Utc::now() - Duration::hours(4)).date_naive()
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn msg(session: &str, key: &str, ts: &str, text: &str) -> IngestMessage {
    IngestMessage {
        provider: "claude".into(),
        session_id: session.into(),
        message_key: key.into(),
        uuid: Some(key.into()),
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
    }
}

/// Ingest one session (with one message stamped `ts`) into `hub.project`, so its
/// logical date is derived from `ts`. Returns the surrogate session id.
async fn ingest_session(hub: &TestHub, session_id: &str, ts: &str, text: &str) -> i64 {
    let batch = IngestBatch {
        machine: MachineInfo {
            machine_id: hub.machine_id,
            hostname: hub.hostname.clone(),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: hub.project.clone(),
            name: Some("proj".into()),
            storage_type: Some("jsonl".into()),
            session_count: Some(1),
            message_count: Some(1),
            last_modified: None,
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: session_id.into(),
            project_path: Some(hub.project.clone()),
            file_path: Some(format!("/tmp/{session_id}.jsonl")),
            entrypoint: None,
            summary: Some("a session".into()),
            message_count: Some(1),
            // Set explicitly AND via the message, so the logical date is the
            // intended one regardless of whether the hub derives it.
            first_message_time: Some(ts.into()),
            last_message_time: Some(ts.into()),
            last_modified: None,
            has_tool_use: Some(false),
            has_errors: Some(false),
            storage_type: Some("jsonl".into()),
        }],
        messages: vec![msg(session_id, &format!("{session_id}-k0"), ts, text)],
    };
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.token)
        .json(&batch)
        .send()
        .await
        .expect("send ingest");
    assert_eq!(resp.status(), 200, "ingest setup must succeed");
    resolve_sid(hub, session_id).await
}

/// Map a provider session id to its hub surrogate id via `/v1/sessions`.
async fn resolve_sid(hub: &TestHub, session_id: &str) -> i64 {
    let body: Value = get(
        hub,
        "/v1/sessions",
        &[("machine", &hub.hostname)],
        Some(&hub.token),
    )
    .await
    .json()
    .await
    .unwrap();
    body.as_array()
        .unwrap()
        .iter()
        .find(|s| s["session_id"].as_str() == Some(session_id))
        .unwrap_or_else(|| panic!("session {session_id} not found in /v1/sessions"))["id"]
        .as_i64()
        .expect("surrogate id must be an integer")
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

async fn post_entry(hub: &TestHub, token: Option<&str>, body: &Value) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(format!("{}/v1/journal/entries", hub.base))
        .json(body);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.unwrap()
}

fn entry_body(
    date: &str,
    project: &str,
    headline: &str,
    summary: &str,
    topics: &[&str],
    session_ids: &[i64],
) -> Value {
    json!({
        "entry_date": date,
        "project_path": project,
        "status": "entry",
        "headline": headline,
        "summary": summary,
        "topics": topics,
        "open_questions": ["what remained unresolved?"],
        "session_ids": session_ids,
        "model": "claude-haiku-4-5",
    })
}

/// The pending groups for THIS test's project (global response, client-filtered).
async fn my_pending(hub: &TestHub, from: &str) -> Vec<Value> {
    let resp = get(
        hub,
        "/v1/journal/pending",
        &[("from", from), ("limit", "200")],
        Some(&hub.token),
    )
    .await;
    assert_eq!(
        resp.status(),
        200,
        "pending must be 200 for a read-authed call"
    );
    let body: Value = resp.json().await.unwrap();
    body.as_array()
        .expect("pending body must be a JSON array")
        .iter()
        .filter(|g| g["project_path"].as_str() == Some(hub.project.as_str()))
        .cloned()
        .collect()
}

/// The browse `entry` rows for THIS test's project.
async fn my_entries(hub: &TestHub) -> Vec<Value> {
    let resp = get(
        hub,
        "/v1/journal/entries",
        &[("project", &hub.project), ("limit", "200")],
        Some(&hub.token),
    )
    .await;
    assert_eq!(
        resp.status(),
        200,
        "browse must be 200 for a read-authed call"
    );
    let body: Value = resp.json().await.unwrap();
    body.as_array()
        .expect("entries body must be a JSON array")
        .iter()
        .filter(|e| e["project_path"].as_str() == Some(hub.project.as_str()))
        .cloned()
        .collect()
}

fn ids_contains(v: &Value, id: i64) -> bool {
    v.as_array()
        .into_iter()
        .flatten()
        .any(|x| x.as_i64() == Some(id))
}

fn has_date(groups: &[Value], date: &str) -> bool {
    groups
        .iter()
        .any(|g| g["entry_date"].as_str() == Some(date))
}

// ---------------------------------------------------------------------------

/// AC1: closed past-day groups are pending, carry their session ids, are
/// ordered newest-first, and `?limit=` is honored.
#[tokio::test]
async fn ac1_pending_lists_closed_groups_ordered_and_limited() {
    let hub = spawn().await;
    let d_old = ymd(days_ago(5));
    let d_new = ymd(days_ago(3));
    let sid_old = ingest_session(&hub, "s-old", &format!("{d_old}T12:00:00Z"), "old day").await;
    let sid_new = ingest_session(&hub, "s-new", &format!("{d_new}T12:00:00Z"), "new day").await;

    let groups = my_pending(&hub, &d_old).await;
    assert_eq!(groups.len(), 2, "both closed groups pending: {groups:?}");

    // Newest-first: the more recent date comes first among my groups.
    assert_eq!(groups[0]["entry_date"].as_str(), Some(d_new.as_str()));
    assert_eq!(groups[1]["entry_date"].as_str(), Some(d_old.as_str()));

    // Each group carries its own session ids.
    assert!(ids_contains(&groups[0]["session_ids"], sid_new));
    assert!(ids_contains(&groups[1]["session_ids"], sid_old));

    // `limit` is honored (caps the page).
    let resp = get(
        &hub,
        "/v1/journal/pending",
        &[("from", &d_old), ("limit", "1")],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let capped: Value = resp.json().await.unwrap();
    assert_eq!(capped.as_array().unwrap().len(), 1, "limit=1 caps the page");
}

/// AC2: the current open logical day is excluded; a closed group from the same
/// project still lists.
#[tokio::test]
async fn ac2_open_day_excluded_closed_day_listed() {
    let hub = spawn().await;
    let closed = ymd(days_ago(4));
    ingest_session(&hub, "s-closed", &format!("{closed}T12:00:00Z"), "closed").await;
    // First message stamped "now" -> current open logical day.
    let now = Utc::now().to_rfc3339();
    ingest_session(&hub, "s-open", &now, "open").await;

    let groups = my_pending(&hub, &closed).await;
    assert!(has_date(&groups, &closed), "closed group must be pending");
    let open = ymd(open_logical_date());
    assert!(
        !has_date(&groups, &open),
        "open logical day {open} must be excluded from pending: {groups:?}"
    );
}

/// AC3: default `day_start_hour = 4 UTC` — a 02:30 session on D+1 and a 23:00
/// session on D fold into a single group dated D.
#[tokio::test]
async fn ac3_logical_day_fold_across_the_4utc_boundary() {
    let hub = spawn().await;
    let d = days_ago(5);
    let d1 = d.succ_opt().unwrap();
    let ds = ymd(d);
    // 02:30 on D+1 shifts back to D; 23:00 on D stays on D.
    let sid_late = ingest_session(&hub, "s-late", &format!("{}T02:30:00Z", ymd(d1)), "late").await;
    let sid_eve = ingest_session(&hub, "s-eve", &format!("{ds}T23:00:00Z"), "evening").await;

    let groups = my_pending(&hub, &ds).await;
    assert_eq!(
        groups.len(),
        1,
        "both sessions fold into one group: {groups:?}"
    );
    assert_eq!(groups[0]["entry_date"].as_str(), Some(ds.as_str()));
    assert!(ids_contains(&groups[0]["session_ids"], sid_late));
    assert!(ids_contains(&groups[0]["session_ids"], sid_eve));
}

/// AC4: POST stores an entry; browse returns it fully; the group leaves pending.
#[tokio::test]
async fn ac4_post_stores_entry_browse_returns_it_pending_clears() {
    let hub = spawn().await;
    let date = ymd(days_ago(3));
    let sid = ingest_session(&hub, "s1", &format!("{date}T10:00:00Z"), "work").await;
    assert!(
        has_date(&my_pending(&hub, &date).await, &date),
        "pending before POST"
    );

    let body = entry_body(
        &date,
        &hub.project,
        "Shipped the widget",
        "We built and shipped the widget across two sessions.",
        &["widget", "shipping", "backend"],
        &[sid],
    );
    let resp = post_entry(&hub, Some(&hub.token), &body).await;
    assert!(
        (200..300).contains(&resp.status().as_u16()),
        "POST entry must be 2xx, got {}",
        resp.status()
    );

    let rows = my_entries(&hub).await;
    assert_eq!(rows.len(), 1, "exactly one browse row: {rows:?}");
    let e = &rows[0];
    assert_eq!(e["headline"].as_str(), Some("Shipped the widget"));
    assert!(e["summary"].as_str().unwrap().contains("widget"));
    assert_eq!(e["topics"].as_array().unwrap().len(), 3);
    assert!(!e["open_questions"].as_array().unwrap().is_empty());
    assert!(ids_contains(&e["session_ids"], sid));

    assert!(
        !has_date(&my_pending(&hub, &date).await, &date),
        "group must leave pending after a fresh entry"
    );
}

/// AC5: re-POSTing the same group replaces content and keeps exactly one row.
#[tokio::test]
async fn ac5_repost_replaces_and_keeps_single_row() {
    let hub = spawn().await;
    let date = ymd(days_ago(3));
    let sid = ingest_session(&hub, "s1", &format!("{date}T10:00:00Z"), "work").await;

    let v1 = entry_body(
        &date,
        &hub.project,
        "First take",
        "v1 summary",
        &["a", "b", "c"],
        &[sid],
    );
    assert!((200..300).contains(
        &post_entry(&hub, Some(&hub.token), &v1)
            .await
            .status()
            .as_u16()
    ));

    let v2 = entry_body(
        &date,
        &hub.project,
        "Second take",
        "v2 summary",
        &["x", "y", "z"],
        &[sid],
    );
    assert!((200..300).contains(
        &post_entry(&hub, Some(&hub.token), &v2)
            .await
            .status()
            .as_u16()
    ));

    let rows = my_entries(&hub).await;
    assert_eq!(rows.len(), 1, "upsert keeps exactly one row: {rows:?}");
    assert_eq!(rows[0]["headline"].as_str(), Some("Second take"));
    assert_eq!(rows[0]["summary"].as_str(), Some("v2 summary"));
}

/// AC6: ingesting a new session for a group with an entry dirties it (pending
/// again); re-POSTing clears it.
#[tokio::test]
async fn ac6_late_session_dirties_entry_then_repost_clears() {
    let hub = spawn().await;
    let date = ymd(days_ago(3));
    let sid1 = ingest_session(&hub, "s1", &format!("{date}T10:00:00Z"), "work").await;

    let v1 = entry_body(
        &date,
        &hub.project,
        "Take one",
        "summary",
        &["a", "b", "c"],
        &[sid1],
    );
    assert!((200..300).contains(
        &post_entry(&hub, Some(&hub.token), &v1)
            .await
            .status()
            .as_u16()
    ));
    assert!(
        !has_date(&my_pending(&hub, &date).await, &date),
        "clean after first POST"
    );

    // A late-arriving session for the same group bumps sessions.updated_at
    // past the entry's generated_at -> dirty.
    let sid2 = ingest_session(&hub, "s2", &format!("{date}T11:00:00Z"), "more work").await;
    assert!(
        has_date(&my_pending(&hub, &date).await, &date),
        "group must be pending again after a late session"
    );

    let v2 = entry_body(
        &date,
        &hub.project,
        "Take two",
        "summary2",
        &["a", "b", "c"],
        &[sid1, sid2],
    );
    assert!((200..300).contains(
        &post_entry(&hub, Some(&hub.token), &v2)
            .await
            .status()
            .as_u16()
    ));
    assert!(
        !has_date(&my_pending(&hub, &date).await, &date),
        "re-distillation must clear the group again"
    );
}

/// AC7: invalid entry payloads are rejected (4xx, not route-missing) and store
/// nothing.
#[tokio::test]
async fn ac7_invalid_payloads_rejected_and_store_nothing() {
    let hub = spawn().await;
    let date = ymd(days_ago(3));
    let sid = ingest_session(&hub, "s1", &format!("{date}T10:00:00Z"), "work").await;
    let bogus_sid = 999_999_999_999_i64;

    // Only 2 topics (needs 3-8).
    let too_few = entry_body(&date, &hub.project, "h", "s", &["one", "two"], &[sid]);
    // References a session id that does not exist.
    let bad_sid = entry_body(
        &date,
        &hub.project,
        "h",
        "s",
        &["a", "b", "c"],
        &[bogus_sid],
    );
    // Unknown status value.
    let bad_status = json!({
        "entry_date": date, "project_path": hub.project, "status": "maybe",
        "headline": "h", "summary": "s", "topics": ["a", "b", "c"],
        "open_questions": [], "session_ids": [sid], "model": "claude-haiku-4-5",
    });

    for body in [&too_few, &bad_sid, &bad_status] {
        let status = post_entry(&hub, Some(&hub.token), body)
            .await
            .status()
            .as_u16();
        assert!(
            (400..500).contains(&status) && status != 404 && status != 405,
            "invalid payload must be a client validation error (400/422), got {status}"
        );
    }

    assert!(
        my_entries(&hub).await.is_empty(),
        "no invalid payload may create a browse row"
    );
}

/// AC8: a skip POST clears the group from pending and never surfaces in browse
/// or search.
#[tokio::test]
async fn ac8_skip_row_hidden_from_browse_and_search() {
    let hub = spawn().await;
    let date = ymd(days_ago(3));
    let tag = hub.machine_id.simple().to_string();
    let term = format!("zqterm{}", &tag[..10]);
    let sid = ingest_session(&hub, "s1", &format!("{date}T10:00:00Z"), &term).await;

    let skip = json!({
        "entry_date": date, "project_path": hub.project, "status": "skip",
        "session_ids": [sid], "model": "claude-haiku-4-5",
    });
    let resp = post_entry(&hub, Some(&hub.token), &skip).await;
    assert!(
        (200..300).contains(&resp.status().as_u16()),
        "skip POST must be 2xx, got {}",
        resp.status()
    );

    assert!(
        !has_date(&my_pending(&hub, &date).await, &date),
        "skip clears pending"
    );
    assert!(
        my_entries(&hub).await.is_empty(),
        "skip never appears in browse"
    );

    // Skip must not surface in journal search for its own project's content.
    let body: Value = get(
        &hub,
        "/v1/search",
        &[("q", &term), ("scope", "all")],
        Some(&hub.token),
    )
    .await
    .json()
    .await
    .unwrap();
    let journal_hits = body["journal"].as_array().cloned().unwrap_or_default();
    assert!(
        !journal_hits
            .iter()
            .any(|h| h["project_path"].as_str() == Some(hub.project.as_str())),
        "skip row must not appear in journal search: {journal_hits:?}"
    );
}

/// AC9: default-scope search returns the existing message `results` AND an
/// additive `journal` array carrying entry fields.
#[tokio::test]
async fn ac9_default_scope_returns_messages_and_journal() {
    let hub = spawn().await;
    let date = ymd(days_ago(3));
    let tag = hub.machine_id.simple().to_string();
    let term = format!("zqterm{}", &tag[..10]);
    let sid = ingest_session(
        &hub,
        "s1",
        &format!("{date}T10:00:00Z"),
        &format!("about {term} today"),
    )
    .await;

    let body = entry_body(
        &date,
        &hub.project,
        &format!("Entry about {term}"),
        &format!("A summary mentioning {term} in detail."),
        &["a", "b", "c"],
        &[sid],
    );
    assert!((200..300).contains(
        &post_entry(&hub, Some(&hub.token), &body)
            .await
            .status()
            .as_u16()
    ));

    let resp = get(
        &hub,
        "/v1/search",
        &[("q", &term), ("machine", &hub.hostname)],
        Some(&hub.token),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();

    // Existing message results are still present, unchanged shape.
    let results = body["results"]
        .as_array()
        .expect("results array must exist");
    assert!(
        results
            .iter()
            .any(|r| r["session_id"].as_str() == Some("s1")),
        "message hit for the seeded term must be present"
    );

    // Additive journal block.
    let journal = body["journal"]
        .as_array()
        .expect("journal array must exist at default scope");
    let hit = journal
        .iter()
        .find(|h| h["project_path"].as_str() == Some(hub.project.as_str()))
        .expect("journal hit for this project must be present");
    assert!(hit["headline"].as_str().unwrap().contains(&term));
    assert_eq!(hit["entry_date"].as_str(), Some(date.as_str()));
    assert!(ids_contains(&hit["session_ids"], sid));
}

/// AC10: `scope=messages` omits the `journal` key entirely; `scope=journal`
/// returns the entry hit and performs no message search.
#[tokio::test]
async fn ac10_scope_messages_and_journal() {
    let hub = spawn().await;
    let date = ymd(days_ago(3));
    let tag = hub.machine_id.simple().to_string();
    let term = format!("zqterm{}", &tag[..10]);
    let sid = ingest_session(
        &hub,
        "s1",
        &format!("{date}T10:00:00Z"),
        &format!("msg {term}"),
    )
    .await;
    let body = entry_body(
        &date,
        &hub.project,
        &format!("Entry {term}"),
        &format!("summary {term}"),
        &["a", "b", "c"],
        &[sid],
    );
    assert!((200..300).contains(
        &post_entry(&hub, Some(&hub.token), &body)
            .await
            .status()
            .as_u16()
    ));

    // scope=messages: no journal key, message hit still present.
    let m: Value = get(
        &hub,
        "/v1/search",
        &[
            ("q", &term),
            ("scope", "messages"),
            ("machine", &hub.hostname),
        ],
        Some(&hub.token),
    )
    .await
    .json()
    .await
    .unwrap();
    assert!(
        m.get("journal").is_none(),
        "scope=messages must omit the journal key: {m}"
    );
    assert!(
        m["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["session_id"].as_str() == Some("s1")),
        "scope=messages still returns message hits"
    );

    // scope=journal: entry hit present, no message hits.
    let j: Value = get(
        &hub,
        "/v1/search",
        &[
            ("q", &term),
            ("scope", "journal"),
            ("machine", &hub.hostname),
        ],
        Some(&hub.token),
    )
    .await
    .json()
    .await
    .unwrap();
    let journal = j["journal"]
        .as_array()
        .expect("scope=journal must carry a journal array");
    assert!(
        journal
            .iter()
            .any(|h| h["project_path"].as_str() == Some(hub.project.as_str())),
        "scope=journal returns the entry hit"
    );
    let no_message_hits = match j.get("results") {
        None => true,
        Some(r) => r.as_array().is_some_and(Vec::is_empty),
    };
    assert!(
        no_message_hits,
        "scope=journal must report no message hits: {j}"
    );
}

/// AC11: reads require a bearer; the write endpoint requires a valid machine
/// token.
#[tokio::test]
async fn ac11_auth_is_enforced() {
    let hub = spawn().await;
    let date = ymd(days_ago(3));

    // Read endpoints reject a missing bearer.
    assert_eq!(
        get(&hub, "/v1/journal/pending", &[], None).await.status(),
        401,
        "GET /v1/journal/pending without a bearer must be 401"
    );
    assert_eq!(
        get(&hub, "/v1/journal/entries", &[], None).await.status(),
        401,
        "GET /v1/journal/entries without a bearer must be 401"
    );

    // Write endpoint rejects missing and invalid machine tokens.
    let body = entry_body(&date, &hub.project, "h", "s", &["a", "b", "c"], &[]);
    assert_eq!(
        post_entry(&hub, None, &body).await.status(),
        401,
        "POST without a token must be 401"
    );
    assert_eq!(
        post_entry(&hub, Some("not-a-real-token"), &body)
            .await
            .status(),
        401,
        "POST with an invalid token must be 401"
    );
}
