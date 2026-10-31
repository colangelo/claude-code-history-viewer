//! Journal read protection (#84 review 3 M2, security-aruba #19;
//! openspec/changes/journal-read-protection-tag): every journal read carries
//! the counted `route`, and every text field of a non-general row starts with
//! the protection tag. Helpers copied from `journal_threads_test.rs`.
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

const TAG: &str = hub::journal::PROTECTION_TAG;

fn project(hub: &TestHub, tag: &str) -> String {
    format!("/w/jtag-{tag}-{}", hub.hostname)
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn get_json(hub: &TestHub, path: &str, query: &[(&str, &str)]) -> (u16, Value) {
    let resp = client()
        .get(format!("{}{path}", hub.base))
        .query(query)
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

/// POST an entry for day `offset` with distinctive text and an optional route.
async fn post(
    hub: &TestHub,
    project: &str,
    s: &HashMap<i64, i64>,
    offset: i64,
    word: &str,
    route: Option<&str>,
) {
    let mut body = json!({
        "entry_date": day(offset).to_string(),
        "project_path": project,
        "status": "entry",
        "headline": format!("{word} headline"),
        "summary": format!("{word} summary"),
        "topics": [format!("{word} topic one"), format!("{word} topic two"), format!("{word} topic three")],
        "open_questions": [format!("{word} question")],
        "session_ids": [s[&offset]],
        "model": "test-model",
    });
    if let Some(r) = route {
        body["route"] = json!(r);
    }
    let resp = client()
        .post(format!("{}/v1/journal/entries", hub.base))
        .bearer_auth(&hub.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "post {word}");
}

/// Every text field of one serialized entry or hit.
fn texts(v: &Value) -> Vec<String> {
    let mut out = vec![
        v["headline"].as_str().unwrap().to_string(),
        v["summary"].as_str().unwrap().to_string(),
    ];
    for k in ["topics", "open_questions"] {
        out.extend(
            v[k].as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string()),
        );
    }
    out
}

fn by_word<'a>(rows: &'a Value, word: &str) -> &'a Value {
    rows.as_array()
        .unwrap()
        .iter()
        .find(|r| r["headline"].as_str().unwrap().contains(word))
        .unwrap_or_else(|| panic!("{word} not in {rows}"))
}

fn assert_tagged(v: &Value, tagged: bool) {
    for t in texts(v) {
        assert_eq!(t.starts_with(TAG), tagged, "{t:?}");
        assert_eq!(
            t.matches(TAG).count(),
            usize::from(tagged),
            "one tag at most: {t:?}"
        );
    }
}

#[tokio::test]
async fn browse_carries_the_counted_route_and_tags_every_non_general_field() {
    let hub = spawn().await;
    let p = project(&hub, "browse");
    let s = seed(&hub, &p, &[5, 4, 3, 2]).await;
    post(&hub, &p, &s, 5, "gamma", Some("general")).await;
    post(&hub, &p, &s, 4, "pi", Some("protected")).await;
    post(&hub, &p, &s, 3, "nu", None).await;
    post(&hub, &p, &s, 2, "sigma", Some("general")).await;
    // A row rewritten by an older hub: route stays, generated_at moves (stale).
    sqlx::query("UPDATE journal_entries SET generated_at = clock_timestamp() WHERE project_path = $1 AND entry_date = $2")
        .bind(&p)
        .bind(day(2))
        .execute(&hub.pool)
        .await
        .unwrap();

    let (status, rows) = get_json(&hub, "/v1/journal/entries", &[("project", &p)]).await;
    assert_eq!(status, 200);
    let g = by_word(&rows, "gamma");
    assert_eq!(g["route"], "general");
    assert_tagged(g, false);
    let pr = by_word(&rows, "pi");
    assert_eq!(pr["route"], "protected");
    assert_tagged(pr, true);
    let nu = by_word(&rows, "nu");
    assert!(nu["route"].is_null() && nu.as_object().unwrap().contains_key("route"));
    assert_tagged(nu, true);
    let stale = by_word(&rows, "sigma");
    assert!(stale["route"].is_null(), "a stale route does not count");
    assert_tagged(stale, true);

    // Stored text and the search text stay untagged.
    let stored: (String, String) = sqlx::query_as(
        "SELECT headline, coalesce(search_text, '') FROM journal_entries WHERE project_path = $1 AND entry_date = $2",
    )
    .bind(&p)
    .bind(day(4))
    .fetch_one(&hub.pool)
    .await
    .unwrap();
    assert!(!stored.0.contains(TAG) && !stored.1.contains(TAG));
}

#[tokio::test]
async fn a_field_that_already_starts_with_the_tag_is_not_tagged_twice() {
    let hub = spawn().await;
    let p = project(&hub, "idem");
    let s = seed(&hub, &p, &[3]).await;
    post(&hub, &p, &s, 3, &format!("{TAG} copied"), Some("protected")).await;
    let (_, rows) = get_json(&hub, "/v1/journal/entries", &[("project", &p)]).await;
    assert_tagged(&rows[0], true);
}

#[tokio::test]
async fn route_general_filters_browse_search_and_the_report() {
    let hub = spawn().await;
    let p = project(&hub, "filt");
    let s = seed(&hub, &p, &[3, 2]).await;
    post(&hub, &p, &s, 3, "kappaword", Some("general")).await;
    post(&hub, &p, &s, 2, "kappaword other", Some("protected")).await;

    let (_, all) = get_json(&hub, "/v1/journal/entries", &[("project", &p)]).await;
    assert_eq!(all.as_array().unwrap().len(), 2);
    let (_, gen) = get_json(
        &hub,
        "/v1/journal/entries",
        &[("project", &p), ("route", "general")],
    )
    .await;
    assert_eq!(gen.as_array().unwrap().len(), 1);
    assert_eq!(gen[0]["route"], "general");
    let (bad, _) = get_json(
        &hub,
        "/v1/journal/entries",
        &[("project", &p), ("route", "protected")],
    )
    .await;
    assert_eq!(bad, 400);

    let q = [
        ("q", "kappaword"),
        ("project", p.as_str()),
        ("scope", "journal"),
    ];
    let (_, all) = get_json(&hub, "/v1/search", &q).await;
    let hits = all["journal"].as_array().unwrap();
    assert_eq!(hits.len(), 2);
    for h in hits {
        assert_tagged(h, h["route"] != "general");
    }
    let mut qg = q.to_vec();
    qg.push(("route", "general"));
    let (_, gen) = get_json(&hub, "/v1/search", &qg).await;
    let hits = gen["journal"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["route"], "general");
    let mut qb = q.to_vec();
    qb.push(("route", "other"));
    assert_eq!(get_json(&hub, "/v1/search", &qb).await.0, 400);
}

#[tokio::test]
async fn the_report_tags_protected_origin_wording_only() {
    let hub = spawn().await;
    let p = project(&hub, "rep");
    let s = seed(&hub, &p, &[3, 2]).await;
    post(&hub, &p, &s, 3, "general-q", Some("general")).await;
    post(&hub, &p, &s, 2, "protected-q", Some("protected")).await;
    let (_, r) = get_json(&hub, "/v1/journal/open-questions", &[("project", &p)]).await;
    for t in r["threads"].as_array().unwrap() {
        let q = t["question"].as_str().unwrap();
        assert_eq!(q.starts_with(TAG), t["origin"] == "protected", "{q:?}");
    }
    let (_, g) = get_json(
        &hub,
        "/v1/journal/open-questions",
        &[("project", &p), ("origin", "general")],
    )
    .await;
    assert_eq!(g["threads"].as_array().unwrap().len(), 1);
    assert!(!g["threads"][0]["question"].as_str().unwrap().contains(TAG));
}

/// C1: a message whose stored text holds the tag (a transcript that quoted a
/// journal read) gets a snippet that starts with the tag, whatever the match.
#[tokio::test]
async fn a_message_snippet_from_tagged_text_starts_with_the_tag() {
    let hub = spawn().await;
    let p = project(&hub, "snip");
    let quoted = format!(
        "earlier tool output: {TAG} northern pipeline summary for the quarter; \
         many words follow here to push the far word out of any fragment that \
         could hold the tag: alpha bravo charlie delta echo foxtrot golf hotel \
         india juliet kilo lima mike november oscar papa quebec romeo zebrafar"
    );
    let plain = "an ordinary message about zebrafar and the protected journal of cchv";
    let batch = IngestBatch {
        machine: MachineInfo {
            machine_id: hub.machine_id,
            hostname: hub.hostname.clone(),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: p.clone(),
            name: Some("jtag".into()),
            storage_type: Some("jsonl".into()),
            ..Default::default()
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: format!("{p}-s"),
            project_path: Some(p.clone()),
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
        }],
        messages: vec![
            IngestMessage {
                search_text: Some(quoted.clone()),
                content: Some(json!([{ "type": "text", "text": quoted }])),
                ..msg(&format!("{p}-s"), "k-tagged", &noon(2))
            },
            IngestMessage {
                search_text: Some(plain.into()),
                content: Some(json!([{ "type": "text", "text": plain }])),
                ..msg(&format!("{p}-s"), "k-plain", &noon(2))
            },
        ],
    };
    let resp = client()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.token)
        .json(&batch)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    for word in ["protected", "journal", "cchv", "zebrafar"] {
        let (status, r) = get_json(
            &hub,
            "/v1/search",
            &[("q", word), ("project", &p), ("scope", "messages")],
        )
        .await;
        assert_eq!(status, 200);
        for hit in r["results"].as_array().unwrap() {
            let snippet = hit["snippet"].as_str().unwrap();
            let tagged = hit["message_key"] == "k-tagged";
            assert_eq!(snippet.starts_with(TAG), tagged, "q={word}: {snippet:?}");
        }
        assert!(
            r["results"]
                .as_array()
                .unwrap()
                .iter()
                .any(|h| h["message_key"] == "k-tagged"),
            "q={word} finds the tagged message"
        );
    }
}
