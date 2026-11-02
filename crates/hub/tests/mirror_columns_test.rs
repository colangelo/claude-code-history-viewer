//! Mirror columns for `/v1/stats/daily` — change `stats-read-api`, tasks 1.1-1.2.
//!
//! `has_content`, `prompt` and `compaction` are computed at mirror sync from the
//! Postgres row (design D1), and an older mirror file is reset once by the
//! schema-version bump so every row carries them.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use hub::config::MirrorConfig;
use hub::mirror::Mirror;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests")
}

async fn pool() -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");
    pool
}

fn temp_cfg(tag: &str) -> MirrorConfig {
    let dir = std::env::temp_dir().join(format!("cchv-mircols-{tag}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    MirrorConfig {
        path: Some(dir.join("stats.duckdb")),
        ..MirrorConfig::default()
    }
}

async fn scaffold(pool: &PgPool, label: &str) -> (Uuid, i64) {
    let machine_id = Uuid::new_v4();
    sqlx::query("INSERT INTO machines (machine_id, hostname) VALUES ($1, $2)")
        .bind(machine_id)
        .bind(format!("host-{label}"))
        .execute(pool)
        .await
        .expect("insert machine");
    let project_id: i64 = sqlx::query_scalar(
        "INSERT INTO projects (machine_id, provider, project_path, name)
         VALUES ($1, 'claude', $2, $3) RETURNING id",
    )
    .bind(machine_id)
    .bind(format!("/tmp/{label}"))
    .bind(label)
    .fetch_one(pool)
    .await
    .expect("insert project");
    let session_id: i64 = sqlx::query_scalar(
        "INSERT INTO sessions (machine_id, provider, session_id, project_id)
         VALUES ($1, 'claude', $2, $3) RETURNING id",
    )
    .bind(machine_id)
    .bind(Uuid::new_v4().to_string())
    .bind(project_id)
    .fetch_one(pool)
    .await
    .expect("insert session");
    (machine_id, session_id)
}

/// One row with the given role, content and raw (`None` content = SQL NULL).
async fn insert_row(
    pool: &PgPool,
    machine_id: Uuid,
    session_id: i64,
    role: Option<&str>,
    content: Option<serde_json::Value>,
    raw: serde_json::Value,
) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO messages
             (machine_id, session_id, uuid, provider, role, "timestamp", content, message_key, raw)
           VALUES ($1, $2, $3, 'claude', $4, '2026-11-02T10:00:00Z'::timestamptz, $5, $3::text, $6)
           RETURNING id"#,
    )
    .bind(machine_id)
    .bind(session_id)
    .bind(Uuid::new_v4())
    .bind(role)
    .bind(content)
    .bind(raw)
    .fetch_one(pool)
    .await
    .expect("insert message")
}

/// `(has_content, prompt, compaction)` of one mirrored row.
fn flags(mirror: &Mirror, id: i64) -> (bool, bool, bool) {
    let conn = mirror.connection().expect("clone connection");
    conn.query_row(
        "SELECT has_content, prompt, compaction FROM messages WHERE id = ?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .expect("row mirrored with the three columns")
}

#[tokio::test]
async fn sync_computes_has_content_prompt_and_compaction() {
    let pool = pool().await;
    let mirror = Mirror::open_or_create(&temp_cfg("flags")).expect("open mirror");
    let (machine, session) = scaffold(&pool, "mircols").await;

    let boundary = insert_row(
        &pool,
        machine,
        session,
        Some("system"),
        Some(json!("Conversation compacted")),
        json!({"type": "system", "subtype": "compact_boundary"}),
    )
    .await;
    let summary = insert_row(
        &pool,
        machine,
        session,
        Some("user"),
        Some(json!(
            "This session is being continued from a previous conversation..."
        )),
        json!({"type": "user"}),
    )
    .await;
    let tool_result_only = insert_row(
        &pool,
        machine,
        session,
        Some("user"),
        Some(json!([{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}])),
        json!({"type": "user"}),
    )
    .await;
    let mixed = insert_row(
        &pool,
        machine,
        session,
        Some("user"),
        Some(json!([
            {"type": "tool_result", "tool_use_id": "t2", "content": "ok"},
            {"type": "text", "text": "and now this"}
        ])),
        json!({"type": "user"}),
    )
    .await;
    let plain_prompt = insert_row(
        &pool,
        machine,
        session,
        Some("user"),
        Some(json!("fix the bug")),
        json!({"type": "user"}),
    )
    .await;
    let no_content_user = insert_row(
        &pool,
        machine,
        session,
        Some("user"),
        None,
        json!({"type": "user"}),
    )
    .await;
    let assistant = insert_row(
        &pool,
        machine,
        session,
        Some("assistant"),
        Some(json!([{"type": "text", "text": "done"}])),
        json!({"type": "assistant"}),
    )
    .await;
    let micro = insert_row(
        &pool,
        machine,
        session,
        Some("system"),
        None,
        json!({"type": "system", "subtype": "microcompact_boundary"}),
    )
    .await;

    mirror.refresh(&pool).await.expect("refresh");

    assert_eq!(flags(&mirror, boundary), (true, false, true), "boundary");
    assert_eq!(
        flags(&mirror, summary),
        (true, true, false),
        "summary prompt"
    );
    assert_eq!(
        flags(&mirror, tool_result_only),
        (true, false, false),
        "a tool-result-only user row is not a prompt"
    );
    assert_eq!(flags(&mirror, mixed), (true, true, false), "mixed user row");
    assert_eq!(
        flags(&mirror, plain_prompt),
        (true, true, false),
        "plain prompt"
    );
    assert_eq!(
        flags(&mirror, no_content_user),
        (false, false, false),
        "no content"
    );
    assert_eq!(flags(&mirror, assistant), (true, false, false), "assistant");
    assert_eq!(
        flags(&mirror, micro),
        (false, false, false),
        "a microcompaction is not a compaction"
    );
}

/// A mirror file written before the columns existed is reset on open, so the
/// next refresh is a cold build that fills them for every row (task 1.2).
#[tokio::test]
async fn an_older_mirror_file_is_reset_by_the_schema_bump() {
    let cfg = temp_cfg("bump");
    let path = cfg.resolved_path();
    {
        // The pre-change shape: messages without the three columns, one row,
        // and no schema version in mirror_meta.
        let conn = duckdb::Connection::open(&path).expect("open raw duckdb");
        conn.execute_batch(
            "CREATE TABLE messages (id BIGINT PRIMARY KEY, session_id BIGINT, role VARCHAR,
                                    conversational BOOLEAN, usage_row BOOLEAN);
             INSERT INTO messages VALUES (42, 1, 'user', true, true);
             CREATE TABLE mirror_meta (k VARCHAR PRIMARY KEY, v VARCHAR);
             INSERT INTO mirror_meta VALUES ('refreshed_at', '2026-11-01T00:00:00+00:00');",
        )
        .expect("write old-shape mirror");
    }

    let mirror = Mirror::open_or_create(&cfg).expect("open mirror");
    assert!(
        mirror.is_empty(),
        "the old rows are gone: next refresh is a cold build"
    );
    assert!(
        !mirror.state().is_ready(),
        "warming until the cold build stamps refreshed_at"
    );
    let conn = mirror.connection().expect("clone connection");
    conn.query_row(
        "SELECT count(has_content) + count(prompt) + count(compaction) FROM messages",
        [],
        |r| r.get::<_, i64>(0),
    )
    .expect("the three columns exist after the reset");
    drop(conn);
    drop(mirror);

    // A second open of the now-current file keeps its rows.
    let pool = pool().await;
    let mirror = Mirror::open_or_create(&cfg).expect("open mirror");
    let (machine, session) = scaffold(&pool, "mircols-bump").await;
    insert_row(
        &pool,
        machine,
        session,
        Some("user"),
        Some(serde_json::json!("hi")),
        serde_json::json!({}),
    )
    .await;
    mirror.refresh(&pool).await.expect("refresh");
    drop(mirror);
    let mirror = Mirror::open_or_create(&cfg).expect("reopen mirror");
    assert!(!mirror.is_empty(), "a current-version file is not reset");
}
