//! Journal open-question threads (Gitea #15): the entry POST's thread links and
//! resolutions, and `GET /v1/journal/open-questions`.
//!
//! The test database is shared, so each test uses its own hostname and project
//! path and asserts only on its own rows. Requires `TEST_DATABASE_URL`/`DATABASE_URL`.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use chrono::{Duration, NaiveDate, Utc};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
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
    pool: PgPool,
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

    let state = hub::AppState::new(pool.clone(), tokens, Vec::new());
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
        pool,
    }
}

/// The logical date `offset` days before the current one.
fn day(offset: i64) -> NaiveDate {
    (Utc::now() - Duration::hours(4) - Duration::days(offset)).date_naive()
}

fn noon(offset: i64) -> String {
    day(offset)
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn msg(session: &str, key: &str, ts: &str) -> IngestMessage {
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
        content: Some(json!([{ "type": "text", "text": key }])),
        raw: json!({ "text": key }),
        search_text: Some(key.into()),
    }
}

/// One session per day offset in `project`, each with one message at noon.
/// Returns `day offset → hub session id`.
async fn seed(hub: &TestHub, project: &str, offsets: &[i64]) -> HashMap<i64, i64> {
    let name = |o: i64| format!("{project}-s{o}");
    let batch = IngestBatch {
        machine: MachineInfo {
            machine_id: hub.machine_id,
            hostname: hub.hostname.clone(),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: project.into(),
            name: Some("jt".into()),
            storage_type: Some("jsonl".into()),
            ..Default::default()
        }],
        sessions: offsets
            .iter()
            .map(|o| IngestSession {
                provider: "claude".into(),
                session_id: name(*o),
                project_path: Some(project.into()),
                file_path: None,
                entrypoint: None,
                summary: None,
                message_count: None,
                first_message_time: None,
                last_message_time: None,
                last_modified: None,
                has_tool_use: Some(false),
                has_errors: Some(false),
                storage_type: Some("jsonl".into()),
            })
            .collect(),
        messages: offsets
            .iter()
            .map(|o| msg(&name(*o), &format!("{}-m", name(*o)), &noon(*o)))
            .collect(),
    };
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.token)
        .json(&batch)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "ingest setup failed");

    let mut ids = HashMap::new();
    for o in offsets {
        let id: i64 = sqlx::query_scalar(
            "SELECT s.id FROM sessions s JOIN machines m ON m.machine_id = s.machine_id \
             WHERE m.machine_id = $1 AND s.session_id = $2",
        )
        .bind(hub.machine_id)
        .bind(name(*o))
        .fetch_one(&hub.pool)
        .await
        .expect("seeded session");
        ids.insert(*o, id);
    }
    ids
}

/// POST an entry for day `offset` with `questions`, and optional links/resolutions.
async fn post(
    hub: &TestHub,
    project: &str,
    sessions: &HashMap<i64, i64>,
    offset: i64,
    questions: &[&str],
    links: Option<Value>,
    resolved: &[i64],
) -> reqwest::Response {
    let mut body = json!({
        "entry_date": day(offset).to_string(),
        "project_path": project,
        "status": "entry",
        "headline": "h",
        "summary": "s",
        "topics": ["one", "two", "three"],
        "open_questions": questions,
        "resolved_threads": resolved,
        "session_ids": [sessions[&offset]],
        "model": "test-model",
    });
    if let Some(l) = links {
        body["open_question_threads"] = l;
    }
    reqwest::Client::new()
        .post(format!("{}/v1/journal/entries", hub.base))
        .bearer_auth(&hub.token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// Stored thread ids of the entry for day `offset`, or `None` when no row exists.
async fn stored_threads(hub: &TestHub, project: &str, offset: i64) -> Option<(Vec<i64>, Vec<i64>)> {
    sqlx::query_as::<_, (Vec<i64>, Vec<i64>)>(
        "SELECT open_question_threads, resolved_threads FROM journal_entries \
         WHERE project_path = $1 AND entry_date = $2",
    )
    .bind(project)
    .bind(day(offset))
    .fetch_optional(&hub.pool)
    .await
    .unwrap()
}

async fn report(hub: &TestHub, query: &[(&str, &str)]) -> Value {
    let resp = reqwest::Client::new()
        .get(format!("{}/v1/journal/open-questions", hub.base))
        .query(query)
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json().await.unwrap()
}

fn thread(report: &Value, id: i64) -> Value {
    report["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["thread_id"] == json!(id))
        .unwrap_or_else(|| panic!("thread {id} not in {report}"))
        .clone()
}

fn project(hub: &TestHub, tag: &str) -> String {
    format!("/w/jt-{tag}-{}", hub.hostname)
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn new_questions_get_ids_and_a_continued_one_keeps_its_own() {
    let hub = spawn().await;
    let p = project(&hub, "link");
    let s = seed(&hub, &p, &[3, 2]).await;

    assert_eq!(
        post(&hub, &p, &s, 3, &["a", "b"], None, &[]).await.status(),
        200
    );
    let (day3, _) = stored_threads(&hub, &p, 3).await.unwrap();
    assert_eq!(day3.len(), 2);
    assert_ne!(day3[0], day3[1]);

    let links = json!([null, day3[1]]);
    assert_eq!(
        post(&hub, &p, &s, 2, &["c", "b again"], Some(links), &[])
            .await
            .status(),
        200
    );
    let (day2, _) = stored_threads(&hub, &p, 2).await.unwrap();
    assert_eq!(day2[1], day3[1]);
    assert!(!day3.contains(&day2[0]), "the new question got a fresh id");

    let r = report(&hub, &[("project", &p)]).await;
    let b = thread(&r, day3[1]);
    assert_eq!(b["question"], "b again");
    assert_eq!(b["days_mentioned"], 2);
    assert_eq!(b["state"], "open");
}

#[tokio::test]
async fn foreign_unknown_and_own_ids_are_rejected_without_a_write() {
    let hub = spawn().await;
    let p = project(&hub, "rej");
    let other = project(&hub, "rej-other");
    let s = seed(&hub, &p, &[3, 2]).await;
    let so = seed(&hub, &other, &[3]).await;
    assert_eq!(
        post(&hub, &other, &so, 3, &["x"], None, &[]).await.status(),
        200
    );
    let (foreign, _) = stored_threads(&hub, &other, 3).await.unwrap();
    assert_eq!(post(&hub, &p, &s, 3, &["a"], None, &[]).await.status(), 200);
    let (own3, _) = stored_threads(&hub, &p, 3).await.unwrap();

    // Another project's id, as a link and as a resolution.
    let r = post(&hub, &p, &s, 2, &["q"], Some(json!([foreign[0]])), &[]).await;
    assert_eq!(r.status(), 400);
    let r = post(&hub, &p, &s, 2, &["q"], None, &[foreign[0]]).await;
    assert_eq!(r.status(), 400);
    // An id nobody has.
    let r = post(&hub, &p, &s, 2, &["q"], Some(json!([i64::MAX])), &[]).await;
    assert_eq!(r.status(), 400);
    assert!(
        stored_threads(&hub, &p, 2).await.is_none(),
        "a rejected POST wrote a row"
    );

    // A re-distill of day 3 may not link to day 3's own (about to be replaced) ids.
    let r = post(&hub, &p, &s, 3, &["a"], Some(json!([own3[0]])), &[]).await;
    assert_eq!(r.status(), 400);
    assert_eq!(stored_threads(&hub, &p, 3).await.unwrap().0, own3);
}

#[tokio::test]
async fn malformed_links_are_rejected() {
    let hub = spawn().await;
    let p = project(&hub, "bad");
    let s = seed(&hub, &p, &[3, 2]).await;
    assert_eq!(post(&hub, &p, &s, 3, &["a"], None, &[]).await.status(), 200);
    let (t, _) = stored_threads(&hub, &p, 3).await.unwrap();

    let r = post(&hub, &p, &s, 2, &["q1", "q2"], Some(json!([null])), &[]).await;
    assert_eq!(r.status(), 400, "length mismatch");
    let r = post(&hub, &p, &s, 2, &["q"], Some(json!([t[0]])), &[t[0]]).await;
    assert_eq!(r.status(), 400, "continued and resolved at once");
    assert!(stored_threads(&hub, &p, 2).await.is_none());
}

#[tokio::test]
async fn resolution_holds_until_a_later_mention_reopens_it() {
    let hub = spawn().await;
    let p = project(&hub, "res");
    let s = seed(&hub, &p, &[4, 3, 2]).await;
    assert_eq!(post(&hub, &p, &s, 4, &["q"], None, &[]).await.status(), 200);
    let (t, _) = stored_threads(&hub, &p, 4).await.unwrap();
    let id = t[0];

    assert_eq!(
        post(&hub, &p, &s, 3, &["other"], None, &[id])
            .await
            .status(),
        200
    );
    let r = report(&hub, &[("project", &p)]).await;
    assert_eq!(thread(&r, id)["state"], "resolved");
    assert_eq!(thread(&r, id)["resolved_on"], json!(day(3).to_string()));

    // `before` = day 3: the resolution is not visible yet.
    let r = report(&hub, &[("project", &p), ("before", &day(3).to_string())]).await;
    assert_eq!(thread(&r, id)["state"], "open");

    let links = json!([id]);
    assert_eq!(
        post(&hub, &p, &s, 2, &["q again"], Some(links), &[])
            .await
            .status(),
        200
    );
    let r = report(&hub, &[("project", &p)]).await;
    assert_eq!(thread(&r, id)["state"], "open");
    assert_eq!(thread(&r, id)["resolved_on"], Value::Null);
}

#[tokio::test]
async fn a_re_distill_replaces_its_links_and_resolutions() {
    let hub = spawn().await;
    let p = project(&hub, "redo");
    let s = seed(&hub, &p, &[3, 2]).await;
    assert_eq!(post(&hub, &p, &s, 3, &["q"], None, &[]).await.status(), 200);
    let id = stored_threads(&hub, &p, 3).await.unwrap().0[0];

    assert_eq!(
        post(&hub, &p, &s, 2, &["x"], None, &[id]).await.status(),
        200
    );
    assert_eq!(
        thread(&report(&hub, &[("project", &p)]).await, id)["state"],
        "resolved"
    );

    // The same day distilled again, this time without the resolution.
    assert_eq!(post(&hub, &p, &s, 2, &["x"], None, &[]).await.status(), 200);
    assert_eq!(
        stored_threads(&hub, &p, 2).await.unwrap().1,
        Vec::<i64>::new()
    );
    assert_eq!(
        thread(&report(&hub, &[("project", &p)]).await, id)["state"],
        "open"
    );
}

#[tokio::test]
async fn an_entry_without_links_still_posts() {
    // What a distiller from before #15 sends: no thread fields at all.
    let hub = spawn().await;
    let p = project(&hub, "old");
    let s = seed(&hub, &p, &[2]).await;
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/journal/entries", hub.base))
        .bearer_auth(&hub.token)
        .json(&json!({
            "entry_date": day(2).to_string(),
            "project_path": p,
            "status": "entry",
            "headline": "h",
            "summary": "s",
            "topics": ["one", "two", "three"],
            "open_questions": ["a", "b", "c"],
            "session_ids": [s[&2]],
            "model": "test-model",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(stored_threads(&hub, &p, 2).await.unwrap().0.len(), 3);
}

#[tokio::test]
async fn state_filter_counts_mode_and_auth() {
    let hub = spawn().await;
    let p = project(&hub, "cnt");
    let s = seed(&hub, &p, &[3, 2]).await;
    assert_eq!(
        post(&hub, &p, &s, 3, &["a", "b"], None, &[]).await.status(),
        200
    );
    let ids = stored_threads(&hub, &p, 3).await.unwrap().0;
    assert_eq!(
        post(&hub, &p, &s, 2, &["c"], None, &[ids[0]])
            .await
            .status(),
        200
    );

    let r = report(&hub, &[("project", &p), ("state", "open")]).await;
    let open: Vec<i64> = r["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["thread_id"].as_i64().unwrap())
        .collect();
    assert!(!open.contains(&ids[0]) && open.contains(&ids[1]));

    let r = report(&hub, &[]).await;
    let mine = r["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["project_path"] == json!(p))
        .expect("project in counts");
    assert_eq!(mine["resolved"], 1);
    assert_eq!(mine["open"], 2);

    let bad = reqwest::Client::new()
        .get(format!("{}/v1/journal/open-questions", hub.base))
        .query(&[("state", "nope")])
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    let anon = reqwest::Client::new()
        .get(format!("{}/v1/journal/open-questions", hub.base))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), 401);
}

// ---------------------------------------------------------------------------
// Thread origin (#84 finding #12, openspec/changes/distill-thread-origin-filter)
// ---------------------------------------------------------------------------

/// POST an entry for day `offset` with an optional `route`.
async fn post_route(
    hub: &TestHub,
    project: &str,
    sessions: &HashMap<i64, i64>,
    offset: i64,
    questions: &[&str],
    links: Option<Value>,
    resolved: &[i64],
    route: Option<&str>,
) -> reqwest::Response {
    let mut body = json!({
        "entry_date": day(offset).to_string(),
        "project_path": project,
        "status": "entry",
        "headline": "h",
        "summary": "s",
        "topics": ["one", "two", "three"],
        "open_questions": questions,
        "resolved_threads": resolved,
        "session_ids": [sessions[&offset]],
        "model": "test-model",
    });
    if let Some(l) = links {
        body["open_question_threads"] = l;
    }
    if let Some(r) = route {
        body["route"] = json!(r);
    }
    reqwest::Client::new()
        .post(format!("{}/v1/journal/entries", hub.base))
        .bearer_auth(&hub.token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// The stored route of day `offset`, and whether it was written with the row.
async fn stored_route(hub: &TestHub, project: &str, offset: i64) -> (Option<String>, bool) {
    sqlx::query_as::<_, (Option<String>, bool)>(
        "SELECT route, coalesce(route_at = generated_at, false) FROM journal_entries \
         WHERE project_path = $1 AND entry_date = $2",
    )
    .bind(project)
    .bind(day(offset))
    .fetch_one(&hub.pool)
    .await
    .unwrap()
}

/// Set day `offset`'s route as if written with the row (bypasses the POST check).
async fn force_route(hub: &TestHub, project: &str, offset: i64, route: &str) {
    sqlx::query(
        "UPDATE journal_entries SET route = $3, route_at = generated_at \
         WHERE project_path = $1 AND entry_date = $2",
    )
    .bind(project)
    .bind(day(offset))
    .bind(route)
    .execute(&hub.pool)
    .await
    .unwrap();
}

fn origin(report: &Value, id: i64) -> Value {
    thread(report, id)["origin"].clone()
}

fn ids_in(report: &Value) -> Vec<i64> {
    report["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["thread_id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn route_is_stored_with_the_row_and_validated() {
    let hub = spawn().await;
    let p = project(&hub, "route");
    let s = seed(&hub, &p, &[4, 3, 2]).await;
    let ok = |r: reqwest::Response| r.status();

    assert_eq!(
        ok(post_route(&hub, &p, &s, 4, &["a"], None, &[], Some("general")).await),
        200
    );
    assert_eq!(
        stored_route(&hub, &p, 4).await,
        (Some("general".into()), true)
    );
    assert_eq!(
        ok(post_route(&hub, &p, &s, 3, &["b"], None, &[], Some("protected")).await),
        200
    );
    assert_eq!(
        stored_route(&hub, &p, 3).await,
        (Some("protected".into()), true)
    );
    assert_eq!(
        ok(post_route(&hub, &p, &s, 2, &["c"], None, &[], None).await),
        200
    );
    assert_eq!(
        stored_route(&hub, &p, 2).await.0,
        None,
        "absent route stores none"
    );

    // M8 f: a re-post without a route stores none over an earlier `general`.
    assert_eq!(
        ok(post_route(&hub, &p, &s, 4, &["a"], None, &[], None).await),
        200
    );
    assert_eq!(stored_route(&hub, &p, 4).await.0, None);

    let before = stored_threads(&hub, &p, 3).await;
    let bad = post_route(&hub, &p, &s, 3, &["x"], None, &[], Some("other")).await;
    assert_eq!(bad.status(), 400);
    assert_eq!(stored_threads(&hub, &p, 3).await, before, "nothing written");
}

#[tokio::test]
async fn origin_is_general_only_when_every_mention_is_general() {
    let hub = spawn().await;
    let p = project(&hub, "orig");
    let s = seed(&hub, &p, &[6, 5, 4, 3, 2]).await;
    // Thread A: general on day 6 and day 5.
    post_route(&hub, &p, &s, 6, &["a", "n"], None, &[], Some("general")).await;
    let six = stored_threads(&hub, &p, 6).await.unwrap().0;
    let (a, n) = (six[0], six[1]);
    post_route(
        &hub,
        &p,
        &s,
        5,
        &["a again"],
        Some(json!([a])),
        &[],
        Some("general"),
    )
    .await;
    // Thread P: protected on day 4, then a later general wording on day 3 (forced past the POST check).
    post_route(&hub, &p, &s, 4, &["p"], None, &[], Some("protected")).await;
    let pid = stored_threads(&hub, &p, 4).await.unwrap().0[0];
    post_route(
        &hub,
        &p,
        &s,
        3,
        &["p general"],
        Some(json!([pid])),
        &[],
        Some("protected"),
    )
    .await;
    force_route(&hub, &p, 3, "general").await;
    // Thread G: general on day 2, then continued by a protected day... posted as no-route (old distiller).
    post_route(&hub, &p, &s, 2, &["g"], None, &[], None).await;
    let g = stored_threads(&hub, &p, 2).await.unwrap().0[0];

    let r = report(&hub, &[("project", &p)]).await;
    assert_eq!(origin(&r, a), "general");
    assert_eq!(origin(&r, n), "general");
    assert_eq!(
        origin(&r, pid),
        "protected",
        "an earlier protected mention keeps it protected"
    );
    assert_eq!(
        origin(&r, g),
        "protected",
        "a mention with no route is protected"
    );
}

#[tokio::test]
async fn a_later_protected_mention_and_an_old_one_both_count() {
    let hub = spawn().await;
    let p = project(&hub, "late");
    let s = seed(&hub, &p, &[40, 4, 3, 2]).await;
    // M8 a: general first, protected later (latest wording protected).
    post_route(&hub, &p, &s, 4, &["t"], None, &[], Some("general")).await;
    let t = stored_threads(&hub, &p, 4).await.unwrap().0[0];
    post_route(
        &hub,
        &p,
        &s,
        3,
        &["t protected"],
        Some(json!([t])),
        &[],
        Some("protected"),
    )
    .await;
    // M8 b: a protected mention older than the 30-day window.
    post_route(&hub, &p, &s, 40, &["old"], None, &[], Some("protected")).await;
    let old = stored_threads(&hub, &p, 40).await.unwrap().0[0];
    post_route(
        &hub,
        &p,
        &s,
        2,
        &["old again"],
        Some(json!([old])),
        &[],
        Some("protected"),
    )
    .await;
    force_route(&hub, &p, 2, "general").await;

    let r = report(&hub, &[("project", &p), ("days", "30")]).await;
    assert_eq!(origin(&r, t), "protected");
    assert_eq!(
        origin(&r, old),
        "protected",
        "the fold reads past the window"
    );
}

#[tokio::test]
async fn a_route_rewritten_without_its_row_counts_as_none() {
    // M6: an older hub after a rollback rewrites the row but not route/route_at.
    let hub = spawn().await;
    let p = project(&hub, "stale");
    let s = seed(&hub, &p, &[3]).await;
    post_route(&hub, &p, &s, 3, &["q"], None, &[], Some("general")).await;
    let q = stored_threads(&hub, &p, 3).await.unwrap().0[0];
    assert_eq!(
        origin(&report(&hub, &[("project", &p)]).await, q),
        "general"
    );
    sqlx::query(
        "UPDATE journal_entries SET generated_at = clock_timestamp() \
         WHERE project_path = $1 AND entry_date = $2",
    )
    .bind(&p)
    .bind(day(3))
    .execute(&hub.pool)
    .await
    .unwrap();
    assert_eq!(
        origin(&report(&hub, &[("project", &p)]).await, q),
        "protected"
    );
}

#[tokio::test]
async fn a_session_shared_with_a_protected_day_taints_its_threads() {
    // M7: day 3 general (thread T), day 2 protected sharing day 3's session.
    let hub = spawn().await;
    let p = project(&hub, "sess");
    let s = seed(&hub, &p, &[3, 2]).await;
    post_route(&hub, &p, &s, 3, &["t"], None, &[], Some("general")).await;
    let t = stored_threads(&hub, &p, 3).await.unwrap().0[0];
    post_route(&hub, &p, &s, 2, &["other"], None, &[], Some("protected")).await;
    assert_eq!(
        origin(&report(&hub, &[("project", &p)]).await, t),
        "general",
        "twin: a protected day sharing no session leaves T general"
    );
    sqlx::query(
        "UPDATE journal_entries SET session_ids = session_ids || $3 \
         WHERE project_path = $1 AND entry_date = $2",
    )
    .bind(&p)
    .bind(day(2))
    .bind(vec![s[&3]])
    .execute(&hub.pool)
    .await
    .unwrap();
    assert_eq!(
        origin(&report(&hub, &[("project", &p)]).await, t),
        "protected"
    );
}

#[tokio::test]
async fn origin_filter_echoes_runs_before_the_limit_and_rejects_other_values() {
    let hub = spawn().await;
    let p = project(&hub, "filt");
    let s = seed(&hub, &p, &[5, 4, 3]).await;
    post_route(
        &hub,
        &p,
        &s,
        5,
        &["general one"],
        None,
        &[],
        Some("general"),
    )
    .await;
    let g = stored_threads(&hub, &p, 5).await.unwrap().0[0];
    // Newer protected threads that would fill a limit of 2 first (M8 c).
    post_route(&hub, &p, &s, 4, &["p1", "p2"], None, &[], Some("protected")).await;
    post_route(&hub, &p, &s, 3, &["p3"], None, &[], Some("protected")).await;

    let all = report(&hub, &[("project", &p), ("limit", "2")]).await;
    assert!(all.get("origin").is_none(), "no echo without the filter");
    assert!(
        !ids_in(&all).contains(&g),
        "unfiltered, the limit takes the newer ones"
    );

    let r = report(
        &hub,
        &[("project", &p), ("origin", "general"), ("limit", "2")],
    )
    .await;
    assert_eq!(r["origin"], "general");
    assert_eq!(ids_in(&r), vec![g]);

    let bad = reqwest::Client::new()
        .get(format!("{}/v1/journal/open-questions", hub.base))
        .query(&[("project", p.as_str()), ("origin", "protected")])
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
}

#[tokio::test]
async fn a_general_entry_cannot_link_or_resolve_a_protected_thread() {
    // M5 layer 1.
    let hub = spawn().await;
    let p = project(&hub, "post");
    let s = seed(&hub, &p, &[3, 2]).await;
    post_route(
        &hub,
        &p,
        &s,
        3,
        &["secret-ish"],
        None,
        &[],
        Some("protected"),
    )
    .await;
    let t = stored_threads(&hub, &p, 3).await.unwrap().0[0];

    let r = post_route(
        &hub,
        &p,
        &s,
        2,
        &["x"],
        Some(json!([t])),
        &[],
        Some("general"),
    )
    .await;
    assert_eq!(r.status(), 400);
    assert_eq!(stored_threads(&hub, &p, 2).await, None, "nothing written");
    let r = post_route(&hub, &p, &s, 2, &["x"], None, &[t], Some("general")).await;
    assert_eq!(r.status(), 400, "resolving it is refused too");
    assert_eq!(stored_threads(&hub, &p, 2).await, None);

    let r = post_route(
        &hub,
        &p,
        &s,
        2,
        &["x"],
        Some(json!([t])),
        &[],
        Some("protected"),
    )
    .await;
    assert_eq!(r.status(), 200);
    let r = post_route(&hub, &p, &s, 2, &["x"], Some(json!([t])), &[], None).await;
    assert_eq!(
        r.status(),
        200,
        "no route: stored, and the thread stays protected"
    );
    assert_eq!(
        origin(&report(&hub, &[("project", &p)]).await, t),
        "protected"
    );
}

#[tokio::test]
async fn the_read_back_query_finds_a_planted_violation() {
    // M5 layer 3: the SQL run on pg1 at the read-back, independent of the fold.
    let hub = spawn().await;
    let p = project(&hub, "rb");
    let s = seed(&hub, &p, &[3, 2]).await;
    let sql = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/sql/journal-thread-origin-readback.sql"),
    )
    .expect("read-back query file");
    let count = |sql: String, p: String| {
        let pool = hub.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT count(*) FROM ({}) v WHERE v.project_path = $1",
                sql.trim().trim_end_matches(';')
            ))
            .bind(p)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    post_route(&hub, &p, &s, 3, &["q"], None, &[], Some("general")).await;
    let q = stored_threads(&hub, &p, 3).await.unwrap().0[0];
    post_route(
        &hub,
        &p,
        &s,
        2,
        &["q again"],
        Some(json!([q])),
        &[],
        Some("general"),
    )
    .await;
    assert_eq!(count(sql.clone(), p.clone()).await, 0, "a clean chain");

    force_route(&hub, &p, 3, "protected").await;
    assert_eq!(
        count(sql, p).await,
        1,
        "a general row linking a protected mention"
    );
}
