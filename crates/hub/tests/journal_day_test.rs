//! The logical-day fold: which sessions belong to which journal group.
//!
//! A session used to belong to the single day it *started*, so one running from
//! the 19th into the 20th filed its whole content under the 19th and was absent
//! from the 20th's group altogether (#35). These tests pin the message-level
//! fold, the windowed message read the distiller relies on, and the provenance
//! rules that keep the two honest.
//!
//! Read endpoints span every machine in the archive, so each test uses a unique
//! hostname and a unique project path and asserts only on its own rows — the
//! test database is shared. Requires `TEST_DATABASE_URL`/`DATABASE_URL`.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use chrono::{Duration, NaiveDate, Utc};
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

/// The logical date `offset` days before the current one, as `YYYY-MM-DD`.
///
/// Fixtures are anchored to *now* rather than to fixed calendar dates for one
/// blunt reason: `/v1/journal/pending` is ordered newest-first and capped at
/// `MAX_LIMIT` (200), and the test database is shared. Groups on a fixed 2026-03
/// date sort behind every group any other test seeds and fall off the end of the
/// page — which reads as "the fold produced nothing", not as "your rows are on
/// page two". Anchoring to the newest closed day puts them at the top.
///
/// The fold itself is still what is under test: the fixtures below choose only
/// *which calendar day* from this helper, and assert on the *relationships*
/// between message times and groups — a message at 03:59:59 and one at 04:00:00
/// landing on different days is true regardless of which day was picked.
fn day(offset: i64) -> NaiveDate {
    (Utc::now() - Duration::hours(4) - Duration::days(offset)).date_naive()
}

/// An RFC 3339 timestamp at `hh:mm:ss` UTC on the logical day `offset` days back.
///
/// NB the *logical* day: 03:59:59 on day D is a timestamp on D's calendar date
/// but belongs to D-1's group, which is exactly the edge the fold test wants.
fn at(offset: i64, hh: u32, mm: u32, ss: u32) -> String {
    day(offset)
        .and_hms_opt(hh, mm, ss)
        .unwrap()
        .and_utc()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn msg(session: &str, key: &str, seq: i32, ts: &str) -> IngestMessage {
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
        content: Some(json!([{ "type": "text", "text": key }])),
        raw: json!({ "text": key }),
        search_text: Some(key.into()),
    }
}

/// Ingest one project with one session per `(session_name, timestamps)` entry.
///
/// Timestamps are given literally, because the fold is exactly what is under
/// test: a fixture that computed them from `now()` would be asserting against
/// the same arithmetic it is meant to check.
async fn seed(hub: &TestHub, project: &str, sessions: &[(&str, Vec<String>)]) {
    let batch = IngestBatch {
        machine: MachineInfo {
            machine_id: hub.machine_id,
            hostname: hub.hostname.clone(),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: project.into(),
            name: Some("jd".into()),
            storage_type: Some("jsonl".into()),
            ..Default::default()
        }],
        sessions: sessions
            .iter()
            .map(|(name, _)| IngestSession {
                provider: "claude".into(),
                session_id: (*name).into(),
                project_path: Some(project.into()),
                file_path: None,
                entrypoint: None,
                summary: Some(format!("summary of {name}")),
                message_count: None,
                first_message_time: None,
                last_message_time: None,
                last_modified: None,
                has_tool_use: Some(false),
                has_errors: Some(false),
                storage_type: Some("jsonl".into()),
            })
            .collect(),
        messages: sessions
            .iter()
            .flat_map(|(name, stamps)| {
                stamps.iter().enumerate().map(move |(i, ts)| {
                    msg(name, &format!("{name}-k{i}"), i32::try_from(i).unwrap(), ts)
                })
            })
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
}

async fn get(hub: &TestHub, path: &str, query: &[(&str, &str)]) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{}{}", hub.base, path))
        .query(query)
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap()
}

/// The pending groups for `project`, as `(entry_date, session_ids)`.
async fn pending_for(hub: &TestHub, project: &str, from: &str) -> Vec<(String, Vec<i64>)> {
    const LIMIT: usize = 200;
    let resp = get(
        hub,
        "/v1/journal/pending",
        &[("from", from), ("limit", &LIMIT.to_string())],
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let all = body.as_array().unwrap();
    // A FULL page means "the newest LIMIT groups", not "all of them", so every
    // absence assertion built on one is unsound — the row may be on page two.
    // Not hypothetical: this silently broke
    // `idle_day_inside_a_session_span_is_not_a_group` once the shared database
    // held 322 groups in the window, and it failed by accusing the PRODUCT.
    // Refuse to answer rather than answer unsoundly.
    assert!(
        all.len() < LIMIT,
        "pending returned a FULL page ({LIMIT}) for from={from}: this test's groups \
         may be past the page boundary, so neither presence nor absence can be \
         concluded. The shared test database has accumulated too much data — drop \
         and recreate it (CI runs against a fresh one). Refusing to assert."
    );
    all.iter()
        .filter(|g| g["project_path"] == Value::String(project.to_string()))
        .map(|g| {
            (
                g["entry_date"].as_str().unwrap().to_string(),
                g["session_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_i64().unwrap())
                    .collect(),
            )
        })
        .collect()
}

async fn post_entry(hub: &TestHub, project: &str, date: &str, ids: &[i64]) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{}/v1/journal/entries", hub.base))
        .bearer_auth(&hub.token)
        .json(&json!({
            "entry_date": date,
            "project_path": project,
            "status": "entry",
            "headline": "h",
            "summary": "s",
            "topics": ["one", "two", "three"],
            "open_questions": [],
            "session_ids": ids,
            "model": "test-model",
        }))
        .send()
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// the fold
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spanning_session_belongs_to_both_days() {
    let hub = spawn().await;
    let project = format!("/w/jd-span-{}", hub.hostname);
    // One session, messages on two logical days. Under the old session-start
    // fold this produced ONE group (the 19th) holding both days' work.
    seed(
        &hub,
        &project,
        &[("s-span", vec![at(2, 16, 5, 0), at(1, 22, 43, 0)])],
    )
    .await;

    let groups = pending_for(&hub, &project, &day(3).to_string()).await;
    let dates: Vec<&str> = groups.iter().map(|(d, _)| d.as_str()).collect();
    assert_eq!(
        dates,
        vec![day(1).to_string(), day(2).to_string()],
        "a session spanning midnight must form a group on each of its days"
    );
    // Both groups name the same single session — provenance, not partition.
    assert_eq!(groups[0].1.len(), 1);
    assert_eq!(groups[0].1, groups[1].1);
}

#[tokio::test]
async fn day_with_no_session_start_is_still_a_group() {
    let hub = spawn().await;
    let project = format!("/w/jd-nostart-{}", hub.hostname);
    // Every message on the 20th belongs to a session that began on the 19th.
    // The old fold produced no 20th group at all: consequence 2 of #35, whole
    // project-days silently absent from the journal.
    seed(
        &hub,
        &project,
        &[("s-a", vec![at(2, 10, 0, 0), at(1, 10, 0, 0)])],
    )
    .await;

    // Scoped to the later day only: the session began on the earlier one.
    let groups = pending_for(&hub, &project, &day(1).to_string()).await;
    assert_eq!(groups.len(), 1, "the later day must be its own group");
    assert_eq!(groups[0].0, day(1).to_string());
}

#[tokio::test]
async fn idle_day_inside_a_session_span_is_not_a_group() {
    let hub = spawn().await;
    let project = format!("/w/jd-idle-{}", hub.hostname);
    // A session resumed two days later. Deriving days from the session's
    // first/last bounds — the cheap alternative to scanning messages — would
    // invent a group for the 2nd with nothing in it.
    seed(
        &hub,
        &project,
        &[("s-gap", vec![at(3, 10, 0, 0), at(1, 10, 0, 0)])],
    )
    .await;

    let dates: Vec<String> = pending_for(&hub, &project, &day(4).to_string())
        .await
        .into_iter()
        .map(|(d, _)| d)
        .collect();
    assert_eq!(
        dates,
        vec![day(1).to_string(), day(3).to_string()],
        "the untouched middle day must not appear"
    );
}

#[tokio::test]
async fn fold_hour_moves_late_night_work_to_the_previous_day() {
    let hub = spawn().await;
    let project = format!("/w/jd-fold-{}", hub.hostname);
    // 03:59 folds back, 04:00 does not — the boundary is half-open at the fold
    // hour, which is the one edge a day-shift is easy to get wrong by one.
    seed(
        &hub,
        &project,
        &[
            ("s-before", vec![at(1, 3, 59, 59)]),
            ("s-after", vec![at(1, 4, 0, 0)]),
        ],
    )
    .await;

    let dates: Vec<String> = pending_for(&hub, &project, &day(3).to_string())
        .await
        .into_iter()
        .map(|(d, _)| d)
        .collect();
    // One second apart, two different logical days.
    assert_eq!(
        dates,
        vec![day(1).to_string(), day(2).to_string()],
        "the fold hour is the boundary, and it is half-open at 04:00"
    );
}

// ---------------------------------------------------------------------------
// provenance
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spanning_session_is_accepted_for_both_days() {
    let hub = spawn().await;
    let project = format!("/w/jd-accept-{}", hub.hostname);
    seed(
        &hub,
        &project,
        &[("s-span", vec![at(2, 16, 0, 0), at(1, 16, 0, 0)])],
    )
    .await;
    let from = day(3).to_string();
    let groups = pending_for(&hub, &project, &from).await;
    let ids = groups[0].1.clone();

    for date in [day(2).to_string(), day(1).to_string()] {
        let resp = post_entry(&hub, &project, &date, &ids).await;
        assert_eq!(
            resp.status(),
            200,
            "the same session must be valid provenance for both of its days"
        );
    }
    assert!(
        pending_for(&hub, &project, &from).await.is_empty(),
        "both groups drained"
    );
}

#[tokio::test]
async fn session_without_a_message_that_day_is_rejected() {
    let hub = spawn().await;
    let project = format!("/w/jd-reject-{}", hub.hostname);
    seed(
        &hub,
        &project,
        &[
            ("s-early", vec![at(2, 16, 0, 0)]),
            ("s-late", vec![at(1, 16, 0, 0)]),
        ],
    )
    .await;
    let groups = pending_for(&hub, &project, &day(3).to_string()).await;
    let all: Vec<i64> = groups.iter().flat_map(|(_, ids)| ids.clone()).collect();
    assert_eq!(all.len(), 2);

    // Naming both sessions for the earlier day: one has no message then.
    let resp = post_entry(&hub, &project, &day(2).to_string(), &all).await;
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no message in this"),
        "expected a membership error, got {body:?}"
    );
}

#[tokio::test]
async fn partial_provenance_is_rejected() {
    let hub = spawn().await;
    let project = format!("/w/jd-partial-{}", hub.hostname);
    seed(
        &hub,
        &project,
        &[
            ("s-one", vec![at(1, 10, 0, 0)]),
            ("s-two", vec![at(1, 11, 0, 0)]),
        ],
    )
    .await;
    let groups = pending_for(&hub, &project, &day(2).to_string()).await;
    assert_eq!(groups[0].1.len(), 2, "both sessions are in the day");

    let resp = post_entry(&hub, &project, &day(1).to_string(), &groups[0].1[..1]).await;
    assert_eq!(
        resp.status(),
        400,
        "a partial set must not watermark the group"
    );
}

#[tokio::test]
async fn provenance_drift_makes_a_group_pending_again() {
    let hub = spawn().await;
    let project = format!("/w/jd-drift-{}", hub.hostname);
    seed(
        &hub,
        &project,
        &[
            ("s-one", vec![at(1, 10, 0, 0)]),
            ("s-two", vec![at(1, 11, 0, 0)]),
        ],
    )
    .await;
    let from = day(2).to_string();
    let date = day(1).to_string();
    let groups = pending_for(&hub, &project, &from).await;
    let ids = groups[0].1.clone();
    assert_eq!(post_entry(&hub, &project, &date, &ids).await.status(), 200);
    assert!(pending_for(&hub, &project, &from).await.is_empty());

    // Rewrite the stored provenance to a stale subset — exactly the shape every
    // entry written under the session-start fold is in. Snapshot-based dirtiness
    // cannot see this: no ingest happened.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&test_db_url())
        .await
        .unwrap();
    sqlx::query("UPDATE journal_entries SET session_ids = $1 WHERE project_path = $2")
        .bind(&ids[..1])
        .bind(&project)
        .execute(&pool)
        .await
        .unwrap();

    let after = pending_for(&hub, &project, &from).await;
    assert_eq!(after.len(), 1, "drifted provenance must re-pend the group");
    assert_eq!(
        after[0].1, ids,
        "and it reports the correct set to write back"
    );

    // Re-distilling with the right set clears it, and it stays clear.
    assert_eq!(post_entry(&hub, &project, &date, &ids).await.status(), 200);
    assert!(
        pending_for(&hub, &project, &from).await.is_empty(),
        "a repaired group must not come back"
    );
}

// ---------------------------------------------------------------------------
// the windowed message read the distiller depends on
// ---------------------------------------------------------------------------

#[tokio::test]
async fn messages_can_be_read_one_day_at_a_time() {
    let hub = spawn().await;
    let project = format!("/w/jd-window-{}", hub.hostname);
    seed(
        &hub,
        &project,
        &[(
            "s-span",
            vec![at(2, 16, 0, 0), at(2, 18, 0, 0), at(1, 9, 0, 0)],
        )],
    )
    .await;
    let sid = pending_for(&hub, &project, &day(3).to_string()).await[0].1[0];

    // Unwindowed: the whole session.
    let resp = get(&hub, &format!("/v1/sessions/{sid}/messages"), &[]).await;
    assert_eq!(resp.headers()["x-total-count"], "3");

    // The earlier logical day: [04:00 on it, 04:00 on the next).
    let resp = get(
        &hub,
        &format!("/v1/sessions/{sid}/messages"),
        &[("from", &at(2, 4, 0, 0)), ("to", &at(1, 4, 0, 0))],
    )
    .await;
    assert_eq!(resp.status(), 200);
    // The header must follow the filter: a session total next to a filtered
    // body makes the caller's paging loop read past the end (or stop short).
    assert_eq!(
        resp.headers()["x-total-count"],
        "2",
        "X-Total-Count must be the windowed count"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);
    let earlier = day(2).to_string();
    for m in body.as_array().unwrap() {
        assert!(
            m["timestamp"].as_str().unwrap().starts_with(&earlier),
            "windowed read returned a message outside the window: {m:?}"
        );
    }
}

#[tokio::test]
async fn message_window_bounds_stand_alone_and_reject_garbage() {
    let hub = spawn().await;
    let project = format!("/w/jd-bounds-{}", hub.hostname);
    seed(
        &hub,
        &project,
        &[("s-b", vec![at(2, 16, 0, 0), at(1, 9, 0, 0)])],
    )
    .await;
    let sid = pending_for(&hub, &project, &day(3).to_string()).await[0].1[0];
    let split = at(1, 0, 0, 0);

    // `to` alone.
    let resp = get(
        &hub,
        &format!("/v1/sessions/{sid}/messages"),
        &[("to", &split)],
    )
    .await;
    assert_eq!(resp.headers()["x-total-count"], "1");

    // `from` alone.
    let resp = get(
        &hub,
        &format!("/v1/sessions/{sid}/messages"),
        &[("from", &split)],
    )
    .await;
    assert_eq!(resp.headers()["x-total-count"], "1");

    // A bare date is not RFC 3339 — the same refusal `/v1/search` gives.
    for bad in [("from", "yesterday"), ("to", "2026-03-20")] {
        let resp = get(&hub, &format!("/v1/sessions/{sid}/messages"), &[bad]).await;
        assert_eq!(resp.status(), 400, "bound {bad:?} must 400");
    }
}

// ---------------------------------------------------------------------------
// arrival attribution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn each_day_reports_its_own_arrival_not_the_session_s() {
    // `healthz_journal` decides staleness from "when did this group's data last
    // arrive". Derived from the group's *sessions* — as it was until the arrival
    // moved into the shared fold — a midnight-spanning session hands the earlier
    // day an arrival timestamp belonging to the later day's data. That is the same
    // misattribution the day fold exists to remove, surviving in the one column
    // nobody was looking at, and it hides a genuinely stale day behind fresh data
    // that belongs to the next one.
    let hub = spawn().await;
    let project = format!("/w/jd-arrival-{}", hub.hostname);
    seed(
        &hub,
        &project,
        &[("s-span", vec![at(2, 10, 0, 0), at(1, 10, 0, 0)])],
    )
    .await;

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&test_db_url())
        .await
        .unwrap();
    // The earlier day's message arrived long ago; the later day's just now. One
    // session, so a session-level max() would report "just now" for both.
    sqlx::query(
        r#"UPDATE messages SET created_at = now() - interval '3 days'
           WHERE machine_id = $1 AND "timestamp" < $2"#,
    )
    .bind(hub.machine_id)
    .bind(
        chrono::NaiveDate::parse_from_str(&day(1).to_string(), "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(4, 0, 0)
            .unwrap()
            .and_utc(),
    )
    .execute(&pool)
    .await
    .unwrap();

    let body: Value = reqwest::Client::new()
        .get(format!("{}/v1/healthz/journal", hub.base))
        .query(&[("within_days", "10")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let mine: Vec<&Value> = body["groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["project_path"] == Value::String(project.clone()))
        .collect();
    assert_eq!(mine.len(), 2, "both days evaluated, got {mine:?}");

    let earlier = mine
        .iter()
        .find(|g| g["entry_date"] == Value::String(day(2).to_string()))
        .expect("earlier day present");
    let later = mine
        .iter()
        .find(|g| g["entry_date"] == Value::String(day(1).to_string()))
        .expect("later day present");

    assert_eq!(
        earlier["stale"],
        Value::Bool(true),
        "the earlier day's data arrived 3 days ago and must read stale, not be \
         masked by the later day's arrival on the same session: {earlier:?}"
    );
    assert_eq!(
        later["stale"],
        Value::Bool(false),
        "the later day's data just arrived: {later:?}"
    );
}
