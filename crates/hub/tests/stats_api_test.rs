//! Integration tests for the `/v1/stats/*` endpoints: auth, parameter
//! handling, identity folding across machines, both not-found cases, and the
//! mirror-shaped behaviour the read model introduces — the warming `503`, the
//! staleness headers, `/v1/healthz/stats`, and session references (Gitea #26).
//!
//! These are the ported endpoint tests of change `hub-stats-duckdb-mirror`
//! (task 4.7): same fixtures and same expectations as when the rollups ran
//! against Postgres, now with the mirror in the path. That the expectations did
//! not have to change is the point — the endpoints' contract is unchanged, only
//! where they read from.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use archive_protocol::{IngestBatch, IngestMessage, IngestProject, IngestSession, MachineInfo};
use hub::config::MirrorConfig;
use hub::mirror::Mirror;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
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
    pool: PgPool,
    /// The statistics read model this hub serves from. Held so tests can drive
    /// refreshes explicitly: the endpoints answer from the mirror, so "ingest
    /// then assert" is now "ingest, refresh, then assert".
    mirror: Arc<Mirror>,
    /// Two machines, so identity folding has something to fold.
    machines: Vec<Uuid>,
    root_commit: String,
    /// Run-unique path prefix. Identity expansion matches by PATH, not just by
    /// key, so a generic path would fold in rows written by other tests sharing
    /// the database — the isolation bug this prefix exists to prevent.
    prefix: String,
    /// Run-unique suffix for provider session ids. The test database persists
    /// between runs, and a fixed id would exist on this run's machine *and* on
    /// every earlier run's, turning the by-session-id lookups below from
    /// "found" into "ambiguous" on the second run.
    tag: String,
}

/// A hub with an **empty** mirror attached: `/v1/stats/*` answers 503 until
/// something calls [`TestHub::refresh`]. That is the deployed cold-start shape,
/// so it is the default here rather than a special case.
async fn spawn() -> TestHub {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");

    let machines: Vec<Uuid> = vec![Uuid::new_v4(), Uuid::new_v4()];
    let token = format!("tok-{}", machines[0]);
    let mut tokens = HashMap::new();
    for m in &machines {
        tokens.insert(format!("tok-{m}"), *m);
    }

    // Its own mirror file per test, in a fresh directory: the mirror is derived
    // state keyed to nothing, so sharing one between tests would let one test's
    // refresh decide another's readiness.
    let dir = std::env::temp_dir().join(format!("cchv-api-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mirror = Arc::new(
        Mirror::open_or_create(&MirrorConfig {
            path: Some(dir.join("stats.duckdb")),
            ..MirrorConfig::default()
        })
        .expect("open mirror"),
    );

    let state = hub::AppState::new(pool.clone(), tokens, Vec::new()).with_mirror(mirror.clone());
    let app = hub::router(state, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestHub {
        base: format!("http://{addr}"),
        token,
        pool,
        mirror,
        // Identity keys are global, so a per-run root keeps runs from colliding.
        root_commit: format!("{}00000000", machines[0].simple()),
        prefix: format!("/tmp/stats-api-{}", machines[0].simple()),
        tag: machines[0].simple().to_string(),
        machines,
    }
}

impl TestHub {
    /// Fold everything ingested so far into the mirror.
    async fn refresh(&self) {
        self.mirror
            .refresh(&self.pool)
            .await
            .expect("refresh stats mirror");
    }

    /// A provider session id unique to this run (see [`TestHub::tag`]).
    fn session_id(&self, label: &str) -> String {
        format!("{label}-{}", self.tag)
    }
}

struct Resp {
    status: reqwest::StatusCode,
    headers: HeaderMap,
    body: Value,
}

async fn request(hub: &TestHub, path: &str, token: Option<&str>) -> Resp {
    let mut req = reqwest::Client::new().get(format!("{}{}", hub.base, path));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.expect("request");
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp.json::<Value>().await.unwrap_or(Value::Null);
    Resp {
        status,
        headers,
        body,
    }
}

async fn get(hub: &TestHub, path: &str, token: Option<&str>) -> (reqwest::StatusCode, Value) {
    let r = request(hub, path, token).await;
    (r.status, r.body)
}

fn msg(session: &str, key: &str, ts: &str, mid: &str, input: i64) -> IngestMessage {
    IngestMessage {
        provider: "claude".into(),
        session_id: session.into(),
        message_key: key.into(),
        uuid: Some(format!("uuid-{key}")),
        parent_uuid: None,
        seq: 0,
        timestamp: Some(ts.into()),
        message_type: Some("assistant".into()),
        role: Some("assistant".into()),
        model: Some("claude-opus-5".into()),
        stop_reason: None,
        input_tokens: Some(input),
        output_tokens: Some(0),
        cache_creation_tokens: None,
        cache_read_tokens: None,
        cost_usd: None,
        duration_ms: None,
        is_sidechain: false,
        content: Some(json!([{ "type": "text", "text": "hi" }])),
        raw: json!({ "messageId": mid }),
        search_text: Some("text".into()),
    }
}

/// Ingest one session on `machine`, at `path`, fingerprinted to the shared root
/// commit so both machines' rows fold into one identity.
async fn ingest_on(hub: &TestHub, machine: Uuid, path: &str, session: &str, input: i64) {
    let batch = IngestBatch {
        machine: MachineInfo {
            machine_id: machine,
            hostname: format!("host-{machine}"),
            os: Some("macos".into()),
        },
        projects: vec![IngestProject {
            provider: "claude".into(),
            project_path: path.into(),
            name: Some("shared-proj".into()),
            storage_type: Some("jsonl".into()),
            session_count: Some(1),
            message_count: Some(1),
            last_modified: None,
            git_root_commit: Some(hub.root_commit.clone()),
            ..Default::default()
        }],
        sessions: vec![IngestSession {
            provider: "claude".into(),
            session_id: session.into(),
            project_path: Some(path.into()),
            file_path: Some(format!("{path}/{session}.jsonl")),
            entrypoint: None,
            summary: Some("a session".into()),
            message_count: Some(1),
            first_message_time: None,
            last_message_time: None,
            last_modified: None,
            has_tool_use: Some(false),
            has_errors: Some(false),
            storage_type: Some("jsonl".into()),
        }],
        messages: vec![msg(
            session,
            &format!("k1-{session}"),
            "2026-07-20T10:00:00Z",
            &format!("m-{session}"),
            input,
        )],
    };
    let status = reqwest::Client::new()
        .post(format!("{}/v1/ingest", hub.base))
        .bearer_auth(format!("tok-{machine}"))
        .json(&batch)
        .send()
        .await
        .expect("ingest")
        .status();
    assert_eq!(status, 200, "ingest failed for {machine}");
}

/// The identity key ingest derived for this run's shared root commit.
async fn identity_key(hub: &TestHub) -> String {
    sqlx::query_scalar("SELECT identity_key FROM projects WHERE git_root_commit = $1 LIMIT 1")
        .bind(&hub.root_commit)
        .fetch_one(&hub.pool)
        .await
        .expect("identity derived")
}

#[tokio::test]
async fn stats_endpoints_require_a_valid_token() {
    let hub = spawn().await;
    for path in [
        "/v1/stats/global",
        "/v1/stats/projects/anything",
        "/v1/stats/sessions/1",
    ] {
        let (status, _) = get(&hub, path, None).await;
        assert_eq!(status, 401, "{path} without a token");
        let (status, _) = get(&hub, path, Some("nope")).await;
        assert_eq!(status, 401, "{path} with a bad token");
    }
}

#[tokio::test]
async fn global_returns_the_full_summary_shape() {
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/g1", hub.prefix),
        &hub.session_id("gs1"),
        42,
    )
    .await;
    hub.refresh().await;

    let (status, body) = get(&hub, "/v1/stats/global", Some(&hub.token)).await;
    assert_eq!(status, 200);
    for key in [
        "total_projects",
        "total_sessions",
        "total_messages",
        "total_tokens",
        "date_range",
        "token_distribution",
        "daily_stats",
        "activity_heatmap",
        "most_used_tools",
        "most_used_skills",
        "most_used_subagents",
        "provider_distribution",
        "model_distribution",
        "top_projects",
    ] {
        assert!(body.get(key).is_some(), "missing {key} in global summary");
    }
    assert!(body["total_messages"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn project_stats_fold_across_machines_and_paths() {
    // The identity point (D6): the same repo on two machines at two paths must
    // report as ONE project, not two.
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/repo", hub.prefix),
        &hub.session_id("s-a"),
        100,
    )
    .await;
    ingest_on(
        &hub,
        hub.machines[1],
        &format!("{}/elsewhere/repo", hub.prefix),
        &hub.session_id("s-b"),
        200,
    )
    .await;
    hub.refresh().await;

    let key = identity_key(&hub).await;
    let (status, body) = get(&hub, &format!("/v1/stats/projects/{key}"), Some(&hub.token)).await;
    assert_eq!(status, 200);
    assert_eq!(
        body["total_sessions"].as_u64().unwrap(),
        2,
        "sessions not folded"
    );
    assert_eq!(body["total_messages"].as_u64().unwrap(), 2);
    assert_eq!(
        body["token_distribution"]["input"].as_u64().unwrap(),
        300,
        "tokens not summed across machines"
    );
    assert_eq!(body["project_name"].as_str().unwrap(), "shared-proj");
}

#[tokio::test]
async fn unknown_identity_and_session_are_404_not_empty_success() {
    let hub = spawn().await;
    hub.refresh().await;
    let (status, _) = get(
        &hub,
        "/v1/stats/projects/no-such-identity",
        Some(&hub.token),
    )
    .await;
    assert_eq!(status, 404, "unknown identity must not return zeroed stats");

    let (status, _) = get(&hub, "/v1/stats/sessions/999999999", Some(&hub.token)).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn session_stats_return_that_session() {
    let hub = spawn().await;
    let session = hub.session_id("only");
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/sess", hub.prefix),
        &session,
        77,
    )
    .await;
    hub.refresh().await;
    let pk: i64 =
        sqlx::query_scalar("SELECT id FROM sessions WHERE machine_id = $1 AND session_id = $2")
            .bind(hub.machines[0])
            .bind(&session)
            .fetch_one(&hub.pool)
            .await
            .unwrap();

    let (status, body) = get(&hub, &format!("/v1/stats/sessions/{pk}"), Some(&hub.token)).await;
    assert_eq!(status, 200);
    assert_eq!(body["session_id"].as_str().unwrap(), session);
    assert_eq!(body["total_input_tokens"].as_u64().unwrap(), 77);
    assert_eq!(body["message_count"].as_u64().unwrap(), 1);
}

#[tokio::test]
async fn date_window_narrows_and_is_validated() {
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/win", hub.prefix),
        &hub.session_id("ws"),
        55,
    )
    .await; // 2026-07-20
    hub.refresh().await;

    let key = identity_key(&hub).await;

    // Window containing the message.
    let (status, body) = get(
        &hub,
        &format!("/v1/stats/projects/{key}?from=2026-07-19&to=2026-07-21"),
        Some(&hub.token),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["total_messages"].as_u64().unwrap(), 1);

    // Window excluding it.
    let (status, body) = get(
        &hub,
        &format!("/v1/stats/projects/{key}?from=2026-07-01&to=2026-07-02"),
        Some(&hub.token),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["total_messages"].as_u64().unwrap(), 0);

    // Malformed and contradictory parameters are 400s that name the problem,
    // not 500s from the query engine.
    for bad in [
        "?from=yesterday",
        "?to=2026-13-45",
        "?from=2026-07-20&to=2026-07-01",
        "?tz=Not/A/Zone;DROP",
    ] {
        let (status, _) = get(
            &hub,
            &format!("/v1/stats/projects/{key}{bad}"),
            Some(&hub.token),
        )
        .await;
        assert_eq!(status, 400, "expected 400 for {bad}");
    }
}

#[tokio::test]
async fn timezone_parameter_shifts_the_buckets() {
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/tz", hub.prefix),
        &hub.session_id("tzs"),
        5,
    )
    .await; // 10:00Z
    hub.refresh().await;

    let key = identity_key(&hub).await;
    let (_, utc) = get(&hub, &format!("/v1/stats/projects/{key}"), Some(&hub.token)).await;
    let (_, rome) = get(
        &hub,
        &format!("/v1/stats/projects/{key}?tz=Europe/Rome"),
        Some(&hub.token),
    )
    .await;

    assert_eq!(utc["activity_heatmap"][0]["hour"].as_u64().unwrap(), 10);
    assert_eq!(
        rome["activity_heatmap"][0]["hour"].as_u64().unwrap(),
        12,
        "UTC+2 in July"
    );
}

// --- the mirror-shaped behaviour (tasks 5.1, 5.2, 5.3, 5.4) -----------------

/// Task 5.1. A cold mirror is a `503` with a `Retry-After`, and it heals into a
/// `200` on the next refresh **without a restart** — the deployed shape after a
/// §2b binary swap, where the first build takes about four minutes.
#[tokio::test]
async fn warming_mirror_answers_503_with_retry_after_then_200() {
    let hub = spawn().await;
    let session = hub.session_id("warm");
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/warm", hub.prefix),
        &session,
        9,
    )
    .await;

    // Nothing mirrored yet.
    for path in ["/v1/stats/global", "/v1/stats/projects/anything"] {
        let r = request(&hub, path, Some(&hub.token)).await;
        assert_eq!(r.status, 503, "{path} while warming");
        assert_eq!(
            r.headers
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok()),
            Some(30),
            "{path} must tell the caller when to come back"
        );
        assert!(
            !r.headers.contains_key("x-stats-mirror-refreshed-at"),
            "a warming mirror has no refresh to report"
        );
    }

    hub.refresh().await;

    let (status, body) = get(&hub, "/v1/stats/global", Some(&hub.token)).await;
    assert_eq!(status, 200, "the same process must recover with no restart");
    assert!(body["total_messages"].as_u64().unwrap() >= 1);
}

/// Task 5.2. Staleness rides on headers, not in the body — a body field would
/// mean touching the `history-core` stat types, which breaks `src-tauri` struct
/// literals that `rust-tests.yml` still builds (design D5).
#[tokio::test]
async fn responses_carry_mirror_staleness_headers() {
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/hdr", hub.prefix),
        &hub.session_id("hdr"),
        3,
    )
    .await;
    hub.refresh().await;

    let r = request(&hub, "/v1/stats/global", Some(&hub.token)).await;
    assert_eq!(r.status, 200);
    let refreshed = r
        .headers
        .get("x-stats-mirror-refreshed-at")
        .and_then(|v| v.to_str().ok())
        .expect("refreshed-at header");
    chrono::DateTime::parse_from_rfc3339(refreshed)
        .expect("refreshed-at must be a parseable RFC3339 instant");
    let age: i64 = r
        .headers
        .get("x-stats-mirror-age-seconds")
        .and_then(|v| v.to_str().ok())
        .expect("age header")
        .parse()
        .expect("age must be an integer");
    assert!(
        (0..60).contains(&age),
        "a mirror refreshed moments ago reported an age of {age}s"
    );

    // The body is untouched: no staleness fields leaked into the stat types.
    assert!(r.body.get("mirror_refreshed_at").is_none());
    assert!(r.body.get("mirror_age_seconds").is_none());

    // And the headers are readable cross-origin, which is the only reason they
    // are useful — the webapp calls the hub directly from a browser.
    let exposed = request(&hub, "/v1/stats/global", Some(&hub.token))
        .await
        .headers
        .get("access-control-expose-headers")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        exposed.contains("x-stats-mirror-refreshed-at")
            && exposed.contains("x-stats-mirror-age-seconds"),
        "staleness headers must be CORS-exposed, got {exposed:?}"
    );
}

/// Task 5.3, all three states plus the lag verdict.
///
/// The lag half is the reason this endpoint exists: a refresher that runs on
/// time but silently steps over rows keeps `age_seconds` at zero forever, so age
/// alone cannot tell a healthy mirror from an incomplete one.
#[tokio::test]
async fn healthz_stats_reports_warming_then_ready_then_stale_and_lagging() {
    let hub = spawn().await;
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/hz", hub.prefix),
        &hub.session_id("hz"),
        11,
    )
    .await;

    // 1. Warming: unauthenticated (Gatus sends no token) and 503.
    let (status, body) = get(&hub, "/v1/healthz/stats", None).await;
    assert_eq!(status, 503);
    assert_eq!(body["status"], "warming");
    assert_eq!(body["ready"], false);
    assert!(body["refreshed_at"].is_null());

    // 2. Fresh and complete.
    hub.refresh().await;
    let (status, body) = get(&hub, "/v1/healthz/stats", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["ready"], true);
    assert!(body["age_seconds"].as_i64().unwrap() < 60);
    assert_eq!(
        body["lag_rows"].as_i64().unwrap(),
        0,
        "a just-refreshed mirror is level with Postgres"
    );
    assert_eq!(body["mirror_max_id"], body["postgres_max_id"]);

    // 3. Stale: age past the threshold. Backdating the recorded refresh time
    //    rather than sleeping keeps the test both fast and exact.
    {
        let conn = hub.mirror.connection().expect("mirror connection");
        conn.execute(
            "INSERT OR REPLACE INTO mirror_meta (k, v) VALUES ('refreshed_at', ?)",
            [(chrono::Utc::now() - chrono::Duration::hours(9)).to_rfc3339()],
        )
        .expect("backdate refreshed_at");
    }
    let (status, body) = get(&hub, "/v1/healthz/stats", None).await;
    assert_eq!(status, 503);
    assert_eq!(body["status"], "stale");
    assert!(body["age_seconds"].as_i64().unwrap() > 3600);
    // A threshold wide enough to cover it is green again, so the verdict is the
    // threshold's and not an artifact of the backdating.
    let (status, body) = get(&hub, "/v1/healthz/stats?stale_after_secs=86400", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["status"], "ok");

    // 4. Lagging: rows in Postgres the mirror has not taken in yet.
    hub.refresh().await;
    ingest_on(
        &hub,
        hub.machines[1],
        &format!("{}/hz-late", hub.prefix),
        &hub.session_id("hz-late"),
        13,
    )
    .await;

    let (status, body) = get(&hub, "/v1/healthz/stats", None).await;
    assert_eq!(
        status, 200,
        "lag alone must not alert — under live ingest it is never zero"
    );
    assert_eq!(body["status"], "ok");
    assert!(
        body["lag_rows"].as_i64().unwrap() > 0,
        "the un-mirrored row must show up as lag"
    );

    let (status, body) = get(&hub, "/v1/healthz/stats?max_lag_rows=0", None).await;
    assert_eq!(
        status, 503,
        "an explicit lag budget is what turns it into an alert"
    );
    assert_eq!(body["status"], "lagging");

    // A budget wide enough to cover it stays green, and a malformed one is a
    // 400 naming the parameter rather than a silently ignored check.
    let (status, _) = get(&hub, "/v1/healthz/stats?max_lag_rows=1000000", None).await;
    assert_eq!(status, 200);
    let (status, body) = get(&hub, "/v1/healthz/stats?max_lag_rows=nope", None).await;
    assert_eq!(status, 400);
    assert!(body["error"]
        .as_str()
        .unwrap_or_default()
        .contains("max_lag_rows"));

    // And a refresh clears it.
    hub.refresh().await;
    let (status, body) = get(&hub, "/v1/healthz/stats?max_lag_rows=0", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["lag_rows"].as_i64().unwrap(), 0);
}

/// Task 5.4 / Gitea #26: `/v1/stats/sessions/{id}` takes either identifier, and
/// an unknown one of either kind is a `404` rather than a parse `400`.
#[tokio::test]
async fn session_stats_accept_a_row_id_or_a_provider_session_id() {
    let hub = spawn().await;
    let session = hub.session_id("byref");
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/byref", hub.prefix),
        &session,
        64,
    )
    .await;
    hub.refresh().await;
    let pk: i64 =
        sqlx::query_scalar("SELECT id FROM sessions WHERE machine_id = $1 AND session_id = $2")
            .bind(hub.machines[0])
            .bind(&session)
            .fetch_one(&hub.pool)
            .await
            .unwrap();

    // Known row id and known provider session id must answer identically.
    let (status, by_pk) = get(&hub, &format!("/v1/stats/sessions/{pk}"), Some(&hub.token)).await;
    assert_eq!(status, 200);
    let (status, by_uuid) = get(
        &hub,
        &format!("/v1/stats/sessions/{session}"),
        Some(&hub.token),
    )
    .await;
    assert_eq!(status, 200, "a provider session id must resolve (#26)");
    assert_eq!(by_pk, by_uuid, "the two references name the same session");
    assert_eq!(by_uuid["total_input_tokens"].as_u64().unwrap(), 64);

    // Unknown, both kinds. Neither is a 400 — an unknown UUID used to fail
    // Axum's `Path<i64>` extraction, which is what #26 is about.
    for unknown in [
        "987654321",
        "0195f0aa-dead-beef-cafe-000000000000",
        "not-an-id",
    ] {
        let (status, _) = get(
            &hub,
            &format!("/v1/stats/sessions/{unknown}"),
            Some(&hub.token),
        )
        .await;
        assert_eq!(status, 404, "unknown reference {unknown}");
    }
}

/// The same provider session id on two machines is reported as ambiguous rather
/// than resolved arbitrarily — the browse endpoint's rule, ported so the two
/// surfaces cannot answer differently for the same input.
#[tokio::test]
async fn an_ambiguous_provider_session_id_is_a_400_naming_the_candidates() {
    let hub = spawn().await;
    let shared = hub.session_id("dupe");
    ingest_on(
        &hub,
        hub.machines[0],
        &format!("{}/dupe-a", hub.prefix),
        &shared,
        1,
    )
    .await;
    ingest_on(
        &hub,
        hub.machines[1],
        &format!("{}/dupe-b", hub.prefix),
        &shared,
        2,
    )
    .await;
    hub.refresh().await;

    let (status, body) = get(
        &hub,
        &format!("/v1/stats/sessions/{shared}"),
        Some(&hub.token),
    )
    .await;
    assert_eq!(status, 400);
    let msg = body["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("ambiguous") && msg.contains("candidates"),
        "the error must say which sessions matched, got {msg:?}"
    );
}
