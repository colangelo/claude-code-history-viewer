//! Integration tests for the hub ingest endpoint.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).
//! Each test uses a fresh random `machine_id` so data is isolated within one
//! shared database. Migrations are applied via the crate's `MIGRATOR` (sqlx
//! takes an advisory lock, so concurrent test runs are safe).

use archive_protocol::{
    IngestBatch, IngestMessage, IngestProject, IngestResponse, IngestSession, MachineInfo,
};
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

/// A running test hub: base URL + the token/machine identity it trusts + a pool
/// for direct assertions.
struct TestHub {
    base: String,
    token: String,
    machine_id: Uuid,
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

    let state = hub::AppState::new(pool.clone(), tokens, Vec::new());
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

async fn post_ingest(hub: &TestHub, token: Option<&str>, batch: &IngestBatch) -> reqwest::Response {
    let client = reqwest::Client::new();
    let mut req = client.post(format!("{}/v1/ingest", hub.base)).json(batch);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    req.send().await.expect("send ingest request")
}

/// One project + one session + `n` messages for the given machine.
fn sample_batch(machine_id: Uuid, session: &str, messages: Vec<IngestMessage>) -> IngestBatch {
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
            summary: Some("a session".into()),
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

fn msg(session: &str, key: &str, uuid: Option<&str>, ts: &str, text: &str) -> IngestMessage {
    IngestMessage {
        provider: "claude".into(),
        session_id: session.into(),
        message_key: key.into(),
        uuid: uuid.map(Into::into),
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
        raw: json!({ "uuid": uuid, "text": text, "orig": true }),
        search_text: Some(text.into()),
    }
}

async fn message_count(hub: &TestHub) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM messages WHERE machine_id = $1")
        .bind(hub.machine_id)
        .fetch_one(&hub.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn valid_ingest_persists_and_counts() {
    let hub = spawn().await;
    let batch = sample_batch(
        hub.machine_id,
        "sess-1",
        vec![
            msg(
                "sess-1",
                "k1",
                Some("u1"),
                "2026-01-01T00:00:00Z",
                "hello world",
            ),
            msg(
                "sess-1",
                "k2",
                Some("u2"),
                "2026-01-01T00:01:00Z",
                "second message",
            ),
        ],
    );
    let resp = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(resp.status(), 200);
    let body: IngestResponse = resp.json().await.unwrap();
    assert_eq!(body.projects_inserted, 1);
    assert_eq!(body.sessions_inserted, 1);
    assert_eq!(body.messages_inserted, 2);
    assert_eq!(message_count(&hub).await, 2);
}

#[tokio::test]
async fn missing_token_is_401() {
    let hub = spawn().await;
    let batch = sample_batch(hub.machine_id, "sess-x", vec![]);
    let resp = post_ingest(&hub, None, &batch).await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn invalid_token_is_401() {
    let hub = spawn().await;
    let batch = sample_batch(hub.machine_id, "sess-x", vec![]);
    let resp = post_ingest(&hub, Some("not-a-real-token"), &batch).await;
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn message_for_unknown_session_is_400_with_no_partial_write() {
    let hub = spawn().await;
    // A valid session S1, plus a message referencing an unknown session S2.
    let mut batch = sample_batch(
        hub.machine_id,
        "sess-known",
        vec![msg(
            "sess-known",
            "k1",
            Some("u1"),
            "2026-01-01T00:00:00Z",
            "ok",
        )],
    );
    batch.messages.push(msg(
        "sess-UNKNOWN",
        "k2",
        Some("u2"),
        "2026-01-01T00:02:00Z",
        "orphan",
    ));
    let resp = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(resp.status(), 400);
    // Whole batch rolled back: not even the valid session/message persisted.
    assert_eq!(message_count(&hub).await, 0);
    let machines: i64 = sqlx::query_scalar("SELECT count(*) FROM machines WHERE machine_id = $1")
        .bind(hub.machine_id)
        .fetch_one(&hub.pool)
        .await
        .unwrap();
    assert_eq!(machines, 0, "machine upsert must roll back with the batch");
}

#[tokio::test]
async fn double_post_is_idempotent() {
    let hub = spawn().await;
    let batch = sample_batch(
        hub.machine_id,
        "sess-dup",
        vec![
            msg("sess-dup", "k1", Some("u1"), "2026-01-01T00:00:00Z", "one"),
            msg("sess-dup", "k2", Some("u2"), "2026-01-01T00:01:00Z", "two"),
        ],
    );
    let first: IngestResponse = post_ingest(&hub, Some(&hub.token), &batch)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(first.messages_inserted, 2);

    let second: IngestResponse = post_ingest(&hub, Some(&hub.token), &batch)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(second.messages_inserted, 0);
    assert_eq!(second.messages_skipped, 2);
    assert_eq!(
        message_count(&hub).await,
        2,
        "no duplicate rows after re-post"
    );
}

#[tokio::test]
async fn raw_jsonb_round_trips() {
    let hub = spawn().await;
    let raw = json!({ "nested": { "a": 1, "b": [true, null, "x"] }, "orig": "verbatim" });
    let mut m = msg("sess-raw", "k1", Some("u1"), "2026-01-01T00:00:00Z", "body");
    m.raw = raw.clone();
    let batch = sample_batch(hub.machine_id, "sess-raw", vec![m]);
    let resp = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(resp.status(), 200);

    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT raw FROM messages WHERE machine_id = $1 AND message_key = 'k1'")
            .bind(hub.machine_id)
            .fetch_one(&hub.pool)
            .await
            .unwrap();
    assert_eq!(stored, raw, "raw JSONB must round-trip verbatim");
}

#[tokio::test]
async fn uuidless_provider_dedups_by_content_key() {
    let hub = spawn().await;
    // No uuid; message_key is a content-derived key. Same key twice → 1 row.
    let batch = sample_batch(
        hub.machine_id,
        "sess-nouuid",
        vec![
            msg(
                "sess-nouuid",
                "content-hash-abc",
                None,
                "2026-01-01T00:00:00Z",
                "x",
            ),
            msg(
                "sess-nouuid",
                "content-hash-abc",
                None,
                "2026-01-01T00:00:00Z",
                "x",
            ),
        ],
    );
    let body: IngestResponse = post_ingest(&hub, Some(&hub.token), &batch)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(body.messages_inserted, 1);
    assert_eq!(body.messages_skipped, 1);
    assert_eq!(message_count(&hub).await, 1);
}

#[tokio::test]
async fn aggregates_update_on_reingest() {
    let hub = spawn().await;
    // First batch: 1 message.
    let b1 = sample_batch(
        hub.machine_id,
        "sess-agg",
        vec![msg(
            "sess-agg",
            "k1",
            Some("u1"),
            "2026-01-01T00:00:00Z",
            "first",
        )],
    );
    post_ingest(&hub, Some(&hub.token), &b1).await;

    // Second batch: the same session gains a later message.
    let b2 = sample_batch(
        hub.machine_id,
        "sess-agg",
        vec![msg(
            "sess-agg",
            "k2",
            Some("u2"),
            "2026-01-02T00:00:00Z",
            "second",
        )],
    );
    post_ingest(&hub, Some(&hub.token), &b2).await;

    let (count, last): (i32, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT message_count, last_message_time FROM sessions WHERE machine_id = $1 AND session_id = 'sess-agg'",
    )
    .bind(hub.machine_id)
    .fetch_one(&hub.pool)
    .await
    .unwrap();
    assert_eq!(count, 2, "cumulative message_count reflects both batches");
    assert_eq!(
        last.unwrap().to_rfc3339(),
        "2026-01-02T00:00:00+00:00",
        "last_message_time advanced to the newer message"
    );
}

/// Session aggregates are applied as the batch's DELTA (`message_count + n`,
/// `LEAST`/`GREATEST` on the bounds) instead of being recomputed from
/// `messages` — see the note in `ingest.rs`. A delta gets the cumulative answer
/// wrong in exactly three ways, so pin all three: it must count only the rows
/// actually inserted (not the rows offered), it must let the bounds widen
/// BACKWARDS when a batch carries an older message than anything archived, and
/// a timestamp-less message must not clobber the bounds.
#[tokio::test]
async fn session_aggregates_apply_the_batch_delta() {
    let hub = spawn().await;

    /// `(message_count, first_message_time, last_message_time)`
    async fn agg(
        hub: &TestHub,
    ) -> (
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) {
        sqlx::query_as(
            "SELECT message_count, first_message_time, last_message_time FROM sessions
             WHERE machine_id = $1 AND session_id = 'sess-delta'",
        )
        .bind(hub.machine_id)
        .fetch_one(&hub.pool)
        .await
        .unwrap()
    }

    let b1 = sample_batch(
        hub.machine_id,
        "sess-delta",
        vec![
            msg(
                "sess-delta",
                "k1",
                Some("u1"),
                "2026-01-05T00:00:00Z",
                "mid",
            ),
            msg(
                "sess-delta",
                "k2",
                Some("u2"),
                "2026-01-10T00:00:00Z",
                "late",
            ),
        ],
    );
    post_ingest(&hub, Some(&hub.token), &b1).await;
    let (count, first, last) = agg(&hub).await;
    assert_eq!(count, 2);
    assert_eq!(first.unwrap().to_rfc3339(), "2026-01-05T00:00:00+00:00");
    assert_eq!(last.unwrap().to_rfc3339(), "2026-01-10T00:00:00+00:00");

    // A batch carrying one genuinely new (and OLDER) message plus a replay of an
    // already-archived one: the conflicting row must not be counted, and the
    // lower bound must move backwards.
    let b2 = sample_batch(
        hub.machine_id,
        "sess-delta",
        vec![
            msg(
                "sess-delta",
                "k0",
                Some("u0"),
                "2026-01-01T00:00:00Z",
                "early",
            ),
            msg(
                "sess-delta",
                "k1",
                Some("u1"),
                "2026-01-05T00:00:00Z",
                "mid",
            ),
        ],
    );
    post_ingest(&hub, Some(&hub.token), &b2).await;
    let (count, first, last) = agg(&hub).await;
    assert_eq!(count, 3, "the already-archived message is not re-counted");
    assert_eq!(
        first.unwrap().to_rfc3339(),
        "2026-01-01T00:00:00+00:00",
        "first_message_time widens backwards"
    );
    assert_eq!(last.unwrap().to_rfc3339(), "2026-01-10T00:00:00+00:00");

    // A message with no timestamp counts, but leaves the bounds alone.
    let mut undated = msg(
        "sess-delta",
        "k3",
        Some("u3"),
        "2026-01-07T00:00:00Z",
        "n/a",
    );
    undated.timestamp = None;
    let b3 = sample_batch(hub.machine_id, "sess-delta", vec![undated]);
    post_ingest(&hub, Some(&hub.token), &b3).await;
    let (count, first, last) = agg(&hub).await;
    assert_eq!(count, 4);
    assert_eq!(first.unwrap().to_rfc3339(), "2026-01-01T00:00:00+00:00");
    assert_eq!(
        last.unwrap().to_rfc3339(),
        "2026-01-10T00:00:00+00:00",
        "an undated message must not NULL out the bounds"
    );

    // Replaying a fully-archived batch adds nothing.
    post_ingest(&hub, Some(&hub.token), &b2).await;
    assert_eq!(agg(&hub).await.0, 4, "replay is a no-op for the count");
}

/// `ingest_xid` is the journal's dirty-detection watermark: `pending` reports an
/// entry stale when a session's xid is not visible in the entry's snapshot. So it
/// must be stamped exactly when a session GAINS messages, and must NOT move when
/// an already-archived batch is replayed — otherwise every retry would re-dirty
/// every entry and the distiller would never converge. The stamp rides along with
/// the session aggregates in one statement, so pin it independently of them.
#[tokio::test]
async fn watermark_moves_only_when_a_session_gains_messages() {
    let hub = spawn().await;

    async fn watermark(hub: &TestHub, session: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT ingest_xid::text::bigint FROM sessions
             WHERE machine_id = $1 AND session_id = $2",
        )
        .bind(hub.machine_id)
        .bind(session)
        .fetch_one(&hub.pool)
        .await
        .unwrap()
    }

    let b1 = sample_batch(
        hub.machine_id,
        "sess-xid",
        vec![msg(
            "sess-xid",
            "k1",
            Some("u1"),
            "2026-01-01T00:00:00Z",
            "first",
        )],
    );
    post_ingest(&hub, Some(&hub.token), &b1).await;
    let after_insert = watermark(&hub, "sess-xid").await;

    // Replay the identical batch: every message is already archived, nothing is
    // touched, so the watermark must stay exactly where it was.
    post_ingest(&hub, Some(&hub.token), &b1).await;
    assert_eq!(
        watermark(&hub, "sess-xid").await,
        after_insert,
        "replaying a fully-archived batch must not re-dirty the session"
    );

    // A genuinely new message in the same session moves it forward again.
    let b2 = sample_batch(
        hub.machine_id,
        "sess-xid",
        vec![msg(
            "sess-xid",
            "k2",
            Some("u2"),
            "2026-01-02T00:00:00Z",
            "second",
        )],
    );
    post_ingest(&hub, Some(&hub.token), &b2).await;
    assert!(
        watermark(&hub, "sess-xid").await > after_insert,
        "a batch that adds a message advances the watermark"
    );
}

/// A project's aggregates roll up its sessions' stored counts rather than
/// re-counting `messages` (see the note in `ingest.rs`). The case that rollup
/// could plausibly get wrong is a batch that touches only ONE session of a
/// multi-session project: the sessions this batch did not touch keep their
/// previously-computed `message_count` and must still be summed in.
#[tokio::test]
async fn project_aggregates_sum_sessions_untouched_by_the_batch() {
    let hub = spawn().await;

    // Batch 1 — session A gains 2 messages. `sample_batch` puts every session
    // under the same project (/tmp/proj), which is what makes this a rollup.
    let b1 = sample_batch(
        hub.machine_id,
        "sess-a",
        vec![
            msg("sess-a", "a1", Some("ua1"), "2026-01-01T00:00:00Z", "one"),
            msg("sess-a", "a2", Some("ua2"), "2026-01-01T00:01:00Z", "two"),
        ],
    );
    post_ingest(&hub, Some(&hub.token), &b1).await;

    // Batch 2 — session B gains 3 messages. Session A is absent from this batch,
    // so it is never recomputed here; the project total must still include it.
    let b2 = sample_batch(
        hub.machine_id,
        "sess-b",
        vec![
            msg("sess-b", "b1", Some("ub1"), "2026-01-02T00:00:00Z", "three"),
            msg("sess-b", "b2", Some("ub2"), "2026-01-02T00:01:00Z", "four"),
            msg("sess-b", "b3", Some("ub3"), "2026-01-02T00:02:00Z", "five"),
        ],
    );
    post_ingest(&hub, Some(&hub.token), &b2).await;

    let (sessions, messages): (i32, i32) = sqlx::query_as(
        "SELECT session_count, message_count FROM projects
         WHERE machine_id = $1 AND project_path = '/tmp/proj'",
    )
    .bind(hub.machine_id)
    .fetch_one(&hub.pool)
    .await
    .unwrap();
    assert_eq!(sessions, 2, "both sessions counted");
    assert_eq!(
        messages, 5,
        "project total sums the untouched session (2) and the touched one (3)"
    );

    // Re-ingesting batch 1 inserts nothing new; the rollup must be idempotent
    // and must not drop session B (which this batch does not mention).
    post_ingest(&hub, Some(&hub.token), &b1).await;
    let messages: i32 = sqlx::query_scalar(
        "SELECT message_count FROM projects
         WHERE machine_id = $1 AND project_path = '/tmp/proj'",
    )
    .bind(hub.machine_id)
    .fetch_one(&hub.pool)
    .await
    .unwrap();
    assert_eq!(messages, 5, "replaying an archived batch is a no-op");
}

/// The session aggregate recompute is ONE set-based statement over every
/// touched session, not a per-session loop. The way a `GROUP BY` rewrite fails
/// is by smearing the batch's totals across the sessions, so a batch carrying
/// two sessions must still leave each with its own count and time bounds.
#[tokio::test]
async fn aggregates_are_per_session_within_one_batch() {
    let hub = spawn().await;
    let mut batch = sample_batch(
        hub.machine_id,
        "sess-a",
        vec![
            msg("sess-a", "a1", Some("ua1"), "2026-01-01T00:00:00Z", "a one"),
            msg("sess-a", "a2", Some("ua2"), "2026-01-03T00:00:00Z", "a two"),
            msg("sess-b", "b1", Some("ub1"), "2026-02-01T00:00:00Z", "b one"),
        ],
    );
    // A second session in the SAME batch, under the same project.
    let mut sess_b = batch.sessions[0].clone();
    sess_b.session_id = "sess-b".into();
    sess_b.file_path = Some("/tmp/proj/sess-b.jsonl".into());
    batch.sessions.push(sess_b);

    let resp = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(resp.status(), 200);

    /// `(session_id, message_count, first_message_time, last_message_time)`
    type SessionAggregate = (
        String,
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    let rows: Vec<SessionAggregate> = sqlx::query_as(
        "SELECT session_id, message_count, first_message_time, last_message_time
         FROM sessions WHERE machine_id = $1 ORDER BY session_id",
    )
    .bind(hub.machine_id)
    .fetch_all(&hub.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "sess-a");
    assert_eq!(rows[0].1, 2, "sess-a counts only its own messages");
    assert_eq!(rows[0].2.unwrap().to_rfc3339(), "2026-01-01T00:00:00+00:00");
    assert_eq!(rows[0].3.unwrap().to_rfc3339(), "2026-01-03T00:00:00+00:00");
    assert_eq!(rows[1].0, "sess-b");
    assert_eq!(rows[1].1, 1, "sess-b counts only its own messages");
    assert_eq!(rows[1].2.unwrap().to_rfc3339(), "2026-02-01T00:00:00+00:00");

    // The project aggregate spans both sessions.
    let (sc, mc): (i32, i32) =
        sqlx::query_as("SELECT session_count, message_count FROM projects WHERE machine_id = $1")
            .bind(hub.machine_id)
            .fetch_one(&hub.pool)
            .await
            .unwrap();
    assert_eq!((sc, mc), (2, 3));
}

/// `machines.last_seen` is a coarse liveness heartbeat: back-to-back ingests
/// must NOT each rewrite the row (it was the archive's hottest write), but a
/// changed hostname/os must still land immediately.
#[tokio::test]
async fn machine_heartbeat_is_coalesced_but_facts_propagate() {
    let hub = spawn().await;
    let b1 = sample_batch(
        hub.machine_id,
        "sess-hb",
        vec![msg(
            "sess-hb",
            "k1",
            Some("u1"),
            "2026-01-01T00:00:00Z",
            "one",
        )],
    );
    post_ingest(&hub, Some(&hub.token), &b1).await;

    let seen_after_first: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT last_seen FROM machines WHERE machine_id = $1")
            .bind(hub.machine_id)
            .fetch_one(&hub.pool)
            .await
            .unwrap();

    // A second ingest moments later leaves the heartbeat alone.
    let b2 = sample_batch(
        hub.machine_id,
        "sess-hb",
        vec![msg(
            "sess-hb",
            "k2",
            Some("u2"),
            "2026-01-02T00:00:00Z",
            "two",
        )],
    );
    post_ingest(&hub, Some(&hub.token), &b2).await;

    let (seen_after_second, hostname): (chrono::DateTime<chrono::Utc>, String) =
        sqlx::query_as("SELECT last_seen, hostname FROM machines WHERE machine_id = $1")
            .bind(hub.machine_id)
            .fetch_one(&hub.pool)
            .await
            .unwrap();
    assert_eq!(
        seen_after_second, seen_after_first,
        "a heartbeat inside the coalescing window must not rewrite the row"
    );
    assert_eq!(hostname, "testbox");

    // A renamed machine bypasses the window.
    let mut b3 = sample_batch(
        hub.machine_id,
        "sess-hb",
        vec![msg(
            "sess-hb",
            "k3",
            Some("u3"),
            "2026-01-03T00:00:00Z",
            "three",
        )],
    );
    b3.machine.hostname = "renamedbox".into();
    post_ingest(&hub, Some(&hub.token), &b3).await;

    let hostname: String =
        sqlx::query_scalar("SELECT hostname FROM machines WHERE machine_id = $1")
            .bind(hub.machine_id)
            .fetch_one(&hub.pool)
            .await
            .unwrap();
    assert_eq!(
        hostname, "renamedbox",
        "a changed fact propagates regardless of the heartbeat window"
    );
}

#[tokio::test]
async fn ingest_sanitizes_nul_characters() {
    let hub = spawn().await;
    // Postgres rejects U+0000 in jsonb and TEXT; real transcripts contain it
    // (raw terminal output). The hub must sanitize instead of 500-ing.
    let text = "terminal output\0with a NUL\0byte";
    let batch = sample_batch(
        hub.machine_id,
        "sess-nul",
        vec![msg(
            "sess-nul",
            "k-nul",
            Some("u-nul"),
            "2026-07-03T12:00:00Z",
            text,
        )],
    );

    let resp = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(resp.status(), 200);
    let body: IngestResponse = resp.json().await.expect("parse response");
    assert_eq!(body.messages_inserted, 1);

    let (search_text, raw): (String, serde_json::Value) = sqlx::query_as(
        "SELECT search_text, raw FROM messages WHERE machine_id = $1 AND message_key = 'k-nul'",
    )
    .bind(hub.machine_id)
    .fetch_one(&hub.pool)
    .await
    .expect("stored row");

    assert!(!search_text.contains('\0'), "NUL must not reach TEXT");
    assert!(
        search_text.contains('\u{FFFD}'),
        "NUL replaced, not dropped"
    );
    let raw_str = raw.to_string();
    assert!(!raw_str.contains('\0'), "NUL must not reach jsonb");
    assert!(raw_str.contains('\u{FFFD}'), "raw preserves a marker");
}

/// gitea #7: a message whose `search_text` exceeds Postgres's 1 MiB tsvector
/// limit must not fail the batch — the hub clamps `search_text` at ingest
/// (raw/content keep full fidelity). Hit in practice by Time Machine
/// backfills of old sessions.
#[tokio::test]
async fn oversized_search_text_is_clamped_not_rejected() {
    let hub = spawn().await;
    // Distinct words so the tsvector genuinely grows with input size.
    let mut huge = String::new();
    for i in 0..200_000 {
        use std::fmt::Write;
        write!(huge, "w{i} ").unwrap();
    }
    assert!(
        huge.len() > 1_048_575,
        "test input must exceed the pg limit"
    );
    let batch = sample_batch(
        hub.machine_id,
        "sess-huge",
        vec![msg(
            "sess-huge",
            "k-huge",
            Some("u-huge"),
            "2026-03-27T17:17:03Z",
            &huge,
        )],
    );
    let res = post_ingest(&hub, Some(&hub.token), &batch).await;
    assert_eq!(res.status(), 200, "oversized message must ingest");

    let stored: String = sqlx::query_scalar(
        "SELECT search_text FROM messages WHERE machine_id = $1 AND message_key = 'k-huge'",
    )
    .bind(hub.machine_id)
    .fetch_one(&hub.pool)
    .await
    .expect("row must exist");
    // Pinned to the live clamp (`SEARCH_TEXT_MAX_BYTES`), not the old 512 KiB
    // bound — a stale ceiling would keep passing if the clamp regressed.
    assert!(stored.len() <= 64 * 1024, "search_text must be clamped");
    assert!(stored.starts_with("w0 w1 "), "head of text preserved");
}

// ---------------------------------------------------------------------------
// project identity: fingerprint persistence + identity_key derivation
// ---------------------------------------------------------------------------

const ROOT: &str = "a3f0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8";

fn fp_project(
    path: &str,
    root: Option<&str>,
    remote: Option<&str>,
    worktree: Option<bool>,
) -> IngestProject {
    IngestProject {
        provider: "claude".into(),
        project_path: path.into(),
        name: Some("p".into()),
        git_root_commit: root.map(Into::into),
        git_remote_url: remote.map(Into::into),
        git_is_worktree: worktree,
        ..Default::default()
    }
}

fn projects_only_batch(machine_id: Uuid, projects: Vec<IngestProject>) -> IngestBatch {
    IngestBatch {
        machine: MachineInfo {
            machine_id,
            hostname: "testbox".into(),
            os: Some("macos".into()),
        },
        projects,
        sessions: vec![],
        messages: vec![],
    }
}

#[derive(sqlx::FromRow)]
struct IdentityRow {
    git_root_commit: Option<String>,
    git_remote_url: Option<String>,
    identity_key: Option<String>,
    git_worktree: bool,
}

async fn identity_row(hub: &TestHub, path: &str) -> IdentityRow {
    sqlx::query_as::<_, IdentityRow>(
        "SELECT git_root_commit, git_remote_url, identity_key, git_worktree
         FROM projects WHERE machine_id = $1 AND project_path = $2",
    )
    .bind(hub.machine_id)
    .bind(path)
    .fetch_one(&hub.pool)
    .await
    .expect("project row exists")
}

#[tokio::test]
async fn fingerprint_lands_and_derives_identity_key() {
    let hub = spawn().await;
    // Raw, credentialed remote: the hub must re-normalize before storing.
    let batch = projects_only_batch(
        hub.machine_id,
        vec![fp_project(
            "/tmp/fp-proj",
            Some(ROOT),
            Some("https://user:tok3n@github.com/acme/foo.git"),
            Some(true),
        )],
    );
    assert_eq!(
        post_ingest(&hub, Some(&hub.token), &batch).await.status(),
        200
    );

    let row = identity_row(&hub, "/tmp/fp-proj").await;
    assert_eq!(row.git_root_commit.as_deref(), Some(ROOT));
    assert_eq!(row.git_remote_url.as_deref(), Some("github.com/acme/foo"));
    assert_eq!(
        row.identity_key.as_deref(),
        Some(format!("g:{ROOT}|github.com/acme/foo").as_str())
    );
    assert!(row.git_worktree);
}

#[tokio::test]
async fn absent_facts_never_clobber_stored_fingerprint() {
    let hub = spawn().await;
    let with_facts = projects_only_batch(
        hub.machine_id,
        vec![fp_project(
            "/tmp/fp-retain",
            Some(ROOT),
            Some("git@github.com:acme/foo.git"),
            Some(false),
        )],
    );
    post_ingest(&hub, Some(&hub.token), &with_facts).await;

    // Old daemon / transient capture failure: same project, no facts.
    let without_facts = projects_only_batch(
        hub.machine_id,
        vec![fp_project("/tmp/fp-retain", None, None, None)],
    );
    assert_eq!(
        post_ingest(&hub, Some(&hub.token), &without_facts)
            .await
            .status(),
        200
    );

    let row = identity_row(&hub, "/tmp/fp-retain").await;
    assert_eq!(row.git_root_commit.as_deref(), Some(ROOT));
    assert_eq!(row.git_remote_url.as_deref(), Some("github.com/acme/foo"));
    assert_eq!(
        row.identity_key.as_deref(),
        Some(format!("g:{ROOT}|github.com/acme/foo").as_str())
    );
}

#[tokio::test]
async fn changed_remote_rederives_identity_key() {
    let hub = spawn().await;
    let before = projects_only_batch(
        hub.machine_id,
        vec![fp_project(
            "/tmp/fp-drift",
            Some(ROOT),
            Some("git@github.com:upstream/foo.git"),
            None,
        )],
    );
    post_ingest(&hub, Some(&hub.token), &before).await;

    // Remote drifted (repo re-pointed at the fork).
    let after = projects_only_batch(
        hub.machine_id,
        vec![fp_project(
            "/tmp/fp-drift",
            Some(ROOT),
            Some("git@github.com:colangelo/foo.git"),
            None,
        )],
    );
    post_ingest(&hub, Some(&hub.token), &after).await;

    let row = identity_row(&hub, "/tmp/fp-drift").await;
    assert_eq!(
        row.identity_key.as_deref(),
        Some(format!("g:{ROOT}|github.com/colangelo/foo").as_str())
    );
}

#[tokio::test]
async fn no_fingerprint_means_null_identity() {
    let hub = spawn().await;
    let batch = projects_only_batch(
        hub.machine_id,
        vec![fp_project("/tmp/fp-none", None, None, None)],
    );
    assert_eq!(
        post_ingest(&hub, Some(&hub.token), &batch).await.status(),
        200
    );

    let row = identity_row(&hub, "/tmp/fp-none").await;
    assert_eq!(row.git_root_commit, None);
    assert_eq!(row.git_remote_url, None);
    assert_eq!(row.identity_key, None);
    assert!(!row.git_worktree);
}

#[tokio::test]
async fn invalid_root_is_discarded_remote_only_key() {
    let hub = spawn().await;
    let batch = projects_only_batch(
        hub.machine_id,
        vec![fp_project(
            "/tmp/fp-badroot",
            Some("not-a-hash"),
            Some("git@github.com:acme/foo.git"),
            None,
        )],
    );
    post_ingest(&hub, Some(&hub.token), &batch).await;

    let row = identity_row(&hub, "/tmp/fp-badroot").await;
    assert_eq!(row.git_root_commit, None, "invalid root must not be stored");
    assert_eq!(row.identity_key.as_deref(), Some("r:github.com/acme/foo"));
}
