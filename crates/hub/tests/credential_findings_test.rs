//! Credential findings at ingest, the summary endpoint and the dry run (Gitea #34).
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).
//! Every credential here is a synthetic value generated at runtime. Assertions
//! never format a value into a failure message.

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use chrono::{DateTime, Utc};
use hub::findings::{self, DryRunOptions, Expect};
use hub::redact::Rule;
use serde_json::json;
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

struct TestHub {
    base: String,
    token: String,
    machine_id: Uuid,
    pool: PgPool,
}

async fn spawn(redact_rules: Vec<Rule>) -> TestHub {
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

    let state =
        hub::AppState::new(pool.clone(), tokens, Vec::new()).with_redact_rules(redact_rules);
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

/// A GitHub-shaped token, random per call: `ghp_` + 40 alphanumerics.
fn synthetic_token() -> String {
    format!(
        "ghp_{}{}",
        Uuid::new_v4().simple(),
        &Uuid::new_v4().simple().to_string()[..8]
    )
}

fn msg(session: &str, key: &str, ts: &str, text: &str) -> IngestMessage {
    IngestMessage {
        provider: "claude".into(),
        session_id: session.into(),
        message_key: key.into(),
        uuid: Some(format!("u-{key}")),
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
        raw: json!({ "uuid": format!("u-{key}"), "text": text }),
        search_text: Some(text.into()),
    }
}

fn batch(machine_id: Uuid, session: &str, messages: Vec<IngestMessage>) -> IngestBatch {
    IngestBatch {
        machine: MachineInfo {
            machine_id,
            hostname: "testbox".into(),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: "/tmp/proj".into(),
            name: Some("proj".into()),
            storage_type: Some("jsonl".into()),
            session_count: Some(1),
            message_count: Some(i32::try_from(messages.len()).unwrap_or(0)),
            last_modified: None,
            ..Default::default()
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: session.into(),
            project_path: Some("/tmp/proj".into()),
            file_path: Some(format!("/tmp/proj/{session}.jsonl")),
            entrypoint: None,
            summary: None,
            message_count: Some(i32::try_from(messages.len()).unwrap_or(0)),
            first_message_time: None,
            last_message_time: None,
            last_modified: None,
            has_tool_use: Some(false),
            has_errors: Some(false),
            storage_type: Some("jsonl".into()),
        }],
        messages,
    }
}

async fn post(hub: &TestHub, b: &IngestBatch) {
    let res = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(&hub.token)
        .json(b)
        .send()
        .await
        .expect("send ingest");
    assert_eq!(res.status(), 200);
}

/// Stored `raw`, `content` and `search_text` of one message, as text.
async fn stored(hub: &TestHub, key: &str) -> (i64, String, String, String) {
    let row: (
        i64,
        serde_json::Value,
        Option<serde_json::Value>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT id, raw, content, search_text FROM messages WHERE machine_id = $1 AND uuid = $2",
    )
    .bind(hub.machine_id)
    .bind(format!("u-{key}"))
    .fetch_one(&hub.pool)
    .await
    .expect("stored message");
    (
        row.0,
        row.1.to_string(),
        row.2.map(|c| c.to_string()).unwrap_or_default(),
        row.3.unwrap_or_default(),
    )
}

/// `(field, rule, redacted)` for every finding on a message row, sorted.
async fn findings_for(hub: &TestHub, row_id: i64) -> Vec<(String, String, bool)> {
    sqlx::query_as(
        "SELECT field, rule, redacted FROM credential_findings WHERE message_ref = $1 ORDER BY field, rule",
    )
    .bind(row_id)
    .fetch_all(&hub.pool)
    .await
    .unwrap()
}

fn all_fields(rule: &str, redacted: bool) -> Vec<(String, String, bool)> {
    ["content", "raw", "search_text"]
        .iter()
        .map(|f| ((*f).to_string(), rule.to_string(), redacted))
        .collect()
}

#[tokio::test]
async fn flag_mode_stores_unchanged_and_records_a_finding() {
    let hub = spawn(Vec::new()).await;
    let secret = synthetic_token();
    let text = format!("pushed with {secret} just now");
    post(
        &hub,
        &batch(
            hub.machine_id,
            "s-flag",
            vec![msg("s-flag", "k1", "2026-01-01T00:00:00Z", &text)],
        ),
    )
    .await;

    let (id, raw, content, search) = stored(&hub, "k1").await;
    assert!(raw.contains(&secret), "flag mode must leave raw unchanged");
    assert!(
        content.contains(&secret),
        "flag mode must leave content unchanged"
    );
    assert!(
        search.contains(&secret),
        "flag mode must leave search_text unchanged"
    );
    assert_eq!(findings_for(&hub, id).await, all_fields("prefix", false));
}

#[tokio::test]
async fn redact_mode_stores_the_marker_in_all_three_fields() {
    let hub = spawn(vec![Rule::Prefix]).await;
    let secret = synthetic_token();
    let text = format!("pushed with {secret} just now");
    post(
        &hub,
        &batch(
            hub.machine_id,
            "s-red",
            vec![msg("s-red", "k1", "2026-01-01T00:00:00Z", &text)],
        ),
    )
    .await;

    let (id, raw, content, search) = stored(&hub, "k1").await;
    for (name, field) in [
        ("raw", &raw),
        ("content", &content),
        ("search_text", &search),
    ] {
        assert!(
            !field.contains(&secret),
            "{name} still holds the value in redact mode"
        );
        assert!(
            field.contains("[REDACTED:prefix]"),
            "{name} lacks the marker"
        );
    }
    assert_eq!(findings_for(&hub, id).await, all_fields("prefix", true));
}

#[tokio::test]
async fn resent_batch_adds_no_row_and_no_finding() {
    let hub = spawn(Vec::new()).await;
    let text = format!("token {}", synthetic_token());
    let b = batch(
        hub.machine_id,
        "s-again",
        vec![msg("s-again", "k1", "2026-01-01T00:00:00Z", &text)],
    );
    post(&hub, &b).await;
    post(&hub, &b).await;

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM messages WHERE machine_id = $1")
        .bind(hub.machine_id)
        .fetch_one(&hub.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let (id, ..) = stored(&hub, "k1").await;
    assert_eq!(findings_for(&hub, id).await.len(), 3);
}

/// A Claude Code `attachment` record as the daemon sends it since #46: no
/// content, the payload inside `raw.attachment`.
fn attachment_msg(session: &str, key: &str, payload: Option<serde_json::Value>) -> IngestMessage {
    let mut m = msg(session, key, "2026-01-01T00:00:00Z", "");
    m.message_type = Some("attachment".into());
    m.role = None;
    m.content = None;
    m.search_text = Some(String::new());
    m.raw = json!({ "uuid": format!("u-{key}"), "type": "attachment" });
    if let Some(p) = payload {
        m.raw["attachment"] = p;
    }
    m
}

#[tokio::test]
async fn attachment_payload_in_raw_is_redacted() {
    let hub = spawn(vec![Rule::Prefix]).await;
    let secret = synthetic_token();
    let payload =
        json!({ "type": "hook_additional_context", "content": [format!("export GH={secret}")] });
    post(
        &hub,
        &batch(
            hub.machine_id,
            "s-att",
            vec![attachment_msg("s-att", "k1", Some(payload))],
        ),
    )
    .await;

    let (id, raw, content, _) = stored(&hub, "k1").await;
    assert!(
        !raw.contains(&secret),
        "the attachment payload kept the value"
    );
    assert!(raw.contains("[REDACTED:prefix]"), "raw lacks the marker");
    assert_eq!(content, "", "an attachment row has no content");
    assert_eq!(
        findings_for(&hub, id).await,
        vec![("raw".to_string(), "prefix".to_string(), true)]
    );
}

/// #46 is for new data only: a record archived before #46 (no payload) that
/// comes back under the same key with a payload is NOT stored again and NOT
/// rewritten. The daemon keeps the key payload-free for exactly this.
#[tokio::test]
async fn attachment_payload_does_not_backfill_an_archived_record() {
    let hub = spawn(Vec::new()).await;
    post(
        &hub,
        &batch(
            hub.machine_id,
            "s-old",
            vec![attachment_msg("s-old", "k1", None)],
        ),
    )
    .await;
    post(
        &hub,
        &batch(
            hub.machine_id,
            "s-old",
            vec![attachment_msg(
                "s-old",
                "k1",
                Some(json!({ "type": "file" })),
            )],
        ),
    )
    .await;

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM messages WHERE machine_id = $1")
        .bind(hub.machine_id)
        .fetch_one(&hub.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let (_, raw, ..) = stored(&hub, "k1").await;
    let raw: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(
        raw.get("attachment").is_none(),
        "the archived record was rewritten: {raw}"
    );
}

#[tokio::test]
async fn token_counts_yield_no_finding() {
    let hub = spawn(vec![Rule::Assign, Rule::Prefix, Rule::Bearer, Rule::Pem]).await;
    let text = "input_tokens: 1234, max_tokens=4096, cache_read_tokens: 99 (tokenizer v2)";
    post(
        &hub,
        &batch(
            hub.machine_id,
            "s-neg",
            vec![msg("s-neg", "k1", "2026-01-01T00:00:00Z", text)],
        ),
    )
    .await;

    let (id, raw, ..) = stored(&hub, "k1").await;
    assert!(
        raw.contains("max_tokens=4096"),
        "a negative must be stored unchanged"
    );
    assert_eq!(
        findings_for(&hub, id).await,
        Vec::<(String, String, bool)>::new()
    );
}

#[tokio::test]
async fn summary_counts_without_text() {
    let hub = spawn(Vec::new()).await;
    let secret = synthetic_token();
    let text = format!("Authorization: Bearer {secret}");
    post(
        &hub,
        &batch(
            hub.machine_id,
            "s-sum",
            vec![msg("s-sum", "k1", "2026-01-01T00:00:00Z", &text)],
        ),
    )
    .await;

    let res = reqwest::Client::new()
        .get(format!("{}/v1/findings/summary?since=24h", hub.base))
        .bearer_auth(&hub.token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.text().await.unwrap();
    assert!(!body.contains(&secret), "summary must never carry a value");
    let s: findings::FindingsSummary = serde_json::from_str(&body).unwrap();
    assert!(s.total >= 3);
    assert!(s.by_rule.get("prefix").copied().unwrap_or(0) >= 1);
    assert_eq!(s.detector_version, hub::redact::DETECTOR_VERSION);

    let anon = reqwest::Client::new()
        .get(format!("{}/v1/findings/summary", hub.base))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), 401);
}

/// A pool whose sessions refuse writes, as the CLI builds it.
async fn read_only_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET default_transaction_read_only = on")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&test_db_url())
        .await
        .unwrap()
}

#[tokio::test]
async fn dry_run_reports_locations_never_values() {
    let hub = spawn(Vec::new()).await;
    // A timestamp no other test uses, so `since` isolates this test's rows.
    let base: DateTime<Utc> = "2093-05-01T00:00:00Z".parse().unwrap();
    let run = Uuid::new_v4().simple().to_string();
    let ts = (base
        + chrono::Duration::seconds(i64::from(u16::from_str_radix(&run[..4], 16).unwrap())))
    .to_rfc3339();
    let secret = synthetic_token();
    let session = format!("s-dry-{run}");
    post(
        &hub,
        &batch(
            hub.machine_id,
            &session,
            vec![
                msg(
                    &session,
                    &format!("{run}-hit"),
                    &ts,
                    &format!("export GH_TOKEN={secret}"),
                ),
                msg(
                    &session,
                    &format!("{run}-miss"),
                    &ts,
                    "nothing to see, 4096 tokens",
                ),
            ],
        ),
    )
    .await;
    let (hit_id, ..) = stored(&hub, &format!("{run}-hit")).await;
    let (miss_id, ..) = stored(&hub, &format!("{run}-miss")).await;

    let pool = read_only_pool().await;
    let since = Some(base);

    // Positive controls found: by row id and by session:uuid.
    let report = findings::dry_run(
        &pool,
        &DryRunOptions {
            since,
            rules: Vec::new(),
            expect: vec![
                Expect::Row(hit_id),
                Expect::parse(&format!("{session}:u-{run}-hit")).unwrap(),
            ],
            batch: 1,
        },
    )
    .await
    .expect("dry run on a read-only pool");
    let ours: Vec<_> = report.hits.iter().filter(|h| h.row_id == hit_id).collect();
    assert!(!ours.is_empty(), "the planted value was not reported");
    for h in &ours {
        assert_eq!(h.session_id, session);
        assert_eq!(h.uuid.as_deref(), Some(format!("u-{run}-hit").as_str()));
        assert_eq!(h.machine, "testbox");
        assert!(h.timestamp.is_some());
    }
    assert!(report
        .hits
        .iter()
        .all(|h| h.row_id > 0 && !h.session_id.is_empty()));
    assert!(report.verdict().starts_with("FLOOR"));
    let text = report.render();
    assert!(
        !text.contains(&secret),
        "the report must never carry a value"
    );
    assert!(text.contains(&format!("row={hit_id}")));
    assert!(text.contains("NOT REACHED"));

    // A known location the scan does not hit flips the verdict.
    let missed = findings::dry_run(
        &pool,
        &DryRunOptions {
            since,
            rules: Vec::new(),
            expect: vec![Expect::Row(hit_id), Expect::Row(miss_id)],
            batch: 100,
        },
    )
    .await
    .unwrap();
    assert!(missed.verdict().starts_with("COULD NOT LOOK"));
    assert!(missed.render().contains(&format!("row {miss_id}: MISSED")));

    // A rule filter that excludes the hit also misses it.
    let filtered = findings::dry_run(
        &pool,
        &DryRunOptions {
            since,
            rules: vec![Rule::Pem],
            expect: vec![Expect::Row(hit_id)],
            batch: 100,
        },
    )
    .await
    .unwrap();
    assert!(filtered.hits.iter().all(|h| h.rule == Rule::Pem));
    assert!(filtered.verdict().starts_with("COULD NOT LOOK"));
}

#[tokio::test]
async fn findings_health_pages_on_a_finding_without_auth_or_values() {
    let hub = spawn(Vec::new()).await;
    let get = |since: &'static str| {
        let url = format!("{}/v1/healthz/findings?since={since}", hub.base);
        async move { reqwest::Client::new().get(url).send().await.unwrap() }
    };

    // A window that holds nothing yet: healthy. One second, after a pause long
    // enough that earlier tests' findings have aged out of it.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let quiet = get("1s").await;
    assert_eq!(quiet.status(), 200);
    let body: serde_json::Value = quiet.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["total"], 0);

    let secret = synthetic_token();
    let text = format!("pushed with {secret} just now");
    post(
        &hub,
        &batch(
            hub.machine_id,
            "s-health",
            vec![msg("s-health", "k1", "2026-01-01T00:00:00Z", &text)],
        ),
    )
    .await;

    // Unauthenticated, like every /v1/healthz/*: 503 once a finding lands.
    let paged = get("1h").await;
    assert_eq!(paged.status(), 503);
    let raw = paged.text().await.unwrap();
    assert!(
        !raw.contains(&secret),
        "the health body must never carry a value"
    );
    assert!(!raw.contains("s-health"), "nor a session id");
    let body: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(body["status"], "findings");
    assert!(body["total"].as_i64().unwrap() >= 3);
    assert!(body["by_rule"]["prefix"].as_i64().unwrap() >= 1);

    let bad = get("forever").await;
    assert_eq!(bad.status(), 400);
}

/// `(raw, content, search_text, ingest_xid)` of a row, as text.
async fn row_state(pool: &PgPool, id: i64) -> (String, String, String, String) {
    let row: (
        serde_json::Value,
        Option<serde_json::Value>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT raw, content, search_text, ingest_xid::text FROM messages WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    (
        row.0.to_string(),
        row.1.map(|c| c.to_string()).unwrap_or_default(),
        row.2.unwrap_or_default(),
        row.3.unwrap_or_default(),
    )
}

/// Task 3.3: the real run rewrites exactly the rows the dry run lists for the
/// chosen rules, refuses before writing on every guard, and leaves nothing for
/// a second run.
#[tokio::test]
async fn redact_existing_rewrites_exactly_what_the_dry_run_lists() {
    let hub = spawn(Vec::new()).await;
    let base: DateTime<Utc> = "2094-05-01T00:00:00Z".parse().unwrap();
    // This test counts rows in its window, so clear the window first: it owns
    // 2094 in the test database, and an earlier aborted run leaves rows there.
    sqlx::query(
        r#"DELETE FROM messages WHERE "timestamp" >= '2094-01-01' AND "timestamp" < '2095-01-01'"#,
    )
    .execute(&hub.pool)
    .await
    .unwrap();
    let run = Uuid::new_v4().simple().to_string();
    let ts = (base
        + chrono::Duration::seconds(i64::from(u16::from_str_radix(&run[..4], 16).unwrap())))
    .to_rfc3339();
    let secret = synthetic_token();
    let assigned = format!("pw{}", Uuid::new_v4().simple());
    let mixed_secret = synthetic_token();
    let mixed_assigned = format!("pw{}", Uuid::new_v4().simple());
    let session = format!("s-real-{run}");
    post(
        &hub,
        &batch(
            hub.machine_id,
            &session,
            vec![
                msg(
                    &session,
                    &format!("{run}-prefix"),
                    &ts,
                    &format!("export GH_TOKEN {secret} now"),
                ),
                msg(
                    &session,
                    &format!("{run}-assign"),
                    &ts,
                    &format!("password={assigned}"),
                ),
                msg(
                    &session,
                    &format!("{run}-clean"),
                    &ts,
                    "nothing to see, 4096 tokens",
                ),
                msg(
                    &session,
                    &format!("{run}-mixed"),
                    &ts,
                    &format!("password={mixed_assigned} pushed {mixed_secret}"),
                ),
            ],
        ),
    )
    .await;
    let (prefix_id, ..) = stored(&hub, &format!("{run}-prefix")).await;
    let (assign_id, ..) = stored(&hub, &format!("{run}-assign")).await;
    let (clean_id, ..) = stored(&hub, &format!("{run}-clean")).await;
    let (mixed_id, ..) = stored(&hub, &format!("{run}-mixed")).await;
    let before = [
        row_state(&hub.pool, prefix_id).await,
        row_state(&hub.pool, assign_id).await,
        row_state(&hub.pool, clean_id).await,
    ];
    let unchanged = |what: &str, ok: bool| assert!(ok, "{what}: a refused run changed a row");

    let opts = |rules: Vec<Rule>, expect: Vec<Expect>, max_rows: i64| findings::RedactOptions {
        scan: DryRunOptions {
            since: Some(base),
            rules,
            expect,
            batch: 1,
        },
        max_rows,
    };

    // Refusals, each before any write.
    for (what, o) in [
        (
            "no rule",
            opts(Vec::new(), vec![Expect::Row(prefix_id)], 10),
        ),
        (
            "could not look",
            opts(vec![Rule::Prefix], vec![Expect::Row(clean_id)], 10),
        ),
        (
            "over max rows",
            opts(vec![Rule::Prefix], vec![Expect::Row(prefix_id)], 1),
        ),
    ] {
        assert!(
            findings::redact_existing(&hub.pool, &o).await.is_err(),
            "{what}: the run must refuse"
        );
        unchanged(what, row_state(&hub.pool, prefix_id).await == before[0]);
        unchanged(what, row_state(&hub.pool, assign_id).await == before[1]);
    }

    // The real run, prefix only.
    let o = opts(vec![Rule::Prefix], vec![Expect::Row(prefix_id)], 2);
    let dry = findings::dry_run(&read_only_pool().await, &o.scan)
        .await
        .unwrap();
    let report = findings::redact_existing(&hub.pool, &o)
        .await
        .expect("real run");
    let listed: std::collections::BTreeSet<i64> = dry.hits.iter().map(|h| h.row_id).collect();
    let rewritten: std::collections::BTreeSet<i64> = report.rewritten.iter().copied().collect();
    assert_eq!(
        rewritten, listed,
        "the real run must rewrite exactly the dry run's rows"
    );
    assert_eq!(report.rewritten, vec![prefix_id, mixed_id]);
    assert_eq!(
        report.after_hits, 0,
        "a rescan of the rewritten rows must find nothing"
    );
    assert!(
        !report.render().contains(&secret),
        "the report must never carry a value"
    );

    let (raw, content, search, xid) = row_state(&hub.pool, prefix_id).await;
    for (name, field) in [
        ("raw", &raw),
        ("content", &content),
        ("search_text", &search),
    ] {
        assert!(!field.contains(&secret), "{name} still holds the value");
        assert!(
            field.contains("[REDACTED:prefix]"),
            "{name} lacks the marker"
        );
    }
    assert_ne!(
        xid, before[0].3,
        "the rewritten row must re-dirty its journal day"
    );
    assert_eq!(
        findings_for(&hub, prefix_id).await,
        all_fields("prefix", true)
    );
    // In a rewritten row, only the chosen rule's span changes: the assigned
    // value beside it survives, because assign was not chosen.
    let (raw, content, search, _) = row_state(&hub.pool, mixed_id).await;
    for (name, field) in [
        ("raw", &raw),
        ("content", &content),
        ("search_text", &search),
    ] {
        assert!(
            !field.contains(&mixed_secret),
            "{name}: mixed row still holds the token"
        );
        assert!(
            field.contains(&mixed_assigned),
            "{name}: an unchosen rule's value was removed"
        );
    }
    // A rule not chosen and a clean row are untouched, byte for byte.
    assert!(
        row_state(&hub.pool, assign_id).await == before[1],
        "assign row changed"
    );
    assert!(
        row_state(&hub.pool, clean_id).await == before[2],
        "clean row changed"
    );

    // Idempotent: nothing left for the same run.
    let again = findings::redact_existing(&hub.pool, &opts(vec![Rule::Prefix], Vec::new(), 1))
        .await
        .expect("second run");
    assert!(again.rewritten.is_empty());
}
