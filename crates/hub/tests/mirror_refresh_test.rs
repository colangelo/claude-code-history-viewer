//! Mirror refresh correctness — change `hub-stats-duckdb-mirror`, task 2.7.
//!
//! Everything here defends design D2, whose two halves are the parts of this
//! feature most likely to be silently wrong:
//!
//! * `usage_row` survives appends untouched, because ids are monotonic;
//! * a row that becomes visible *below* the watermark is still picked up,
//!   because refresh re-scans an overlap window.
//!
//! The second is the one a naive `WHERE id > max_id` gets wrong, and it fails
//! invisibly — the mirror simply under-counts forever — so it is asserted
//! directly rather than inferred.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use hub::config::MirrorConfig;
use hub::mirror::{Mirror, MirrorState};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests")
}

/// A mirror in a fresh temp dir, plus a migrated pool.
async fn setup(tag: &str) -> (Mirror, PgPool, MirrorConfig) {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");

    let dir = std::env::temp_dir().join(format!("cchv-mirror-{tag}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let cfg = MirrorConfig {
        path: Some(dir.join("stats.duckdb")),
        ..MirrorConfig::default()
    };
    let mirror = Mirror::open_or_create(&cfg).expect("open mirror");
    (mirror, pool, cfg)
}

/// Insert a machine/project/session scaffold, returning `(machine_id, session_id)`.
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

/// One message row. `message_id` drives the dedup grouping.
async fn insert_message(
    pool: &PgPool,
    machine_id: Uuid,
    session_id: i64,
    message_id: Option<&str>,
    ts: &str,
) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO messages
             (machine_id, session_id, uuid, message_id, provider, role, "timestamp",
              input_tokens, content, message_key, raw)
           VALUES ($5, $1, $2, $3, 'claude', 'assistant', $4::timestamptz, 10, '[]'::jsonb,
                   $2::text, '{}'::jsonb)
           RETURNING id"#,
    )
    .bind(session_id)
    .bind(Uuid::new_v4())
    .bind(message_id)
    .bind(ts)
    .bind(machine_id)
    .fetch_one(pool)
    .await
    .expect("insert message")
}

fn usage_rows(mirror: &Mirror, session_id: i64) -> Vec<(i64, bool)> {
    let conn = mirror.connection().expect("clone connection");
    let mut stmt = conn
        .prepare("SELECT id, usage_row FROM messages WHERE session_id = ? ORDER BY id")
        .expect("prepare");
    let rows = stmt
        .query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query");
    rows.flatten().collect()
}

fn count_for(mirror: &Mirror, session_id: i64) -> i64 {
    let conn = mirror.connection().expect("clone connection");
    conn.query_row(
        "SELECT count(*) FROM messages WHERE session_id = ?",
        [session_id],
        |r| r.get(0),
    )
    .expect("count")
}

/// The everyday case: a second row joining an existing logical message is not
/// usage-bearing, so its repeated usage block cannot be double counted.
#[tokio::test]
async fn later_row_in_an_existing_group_is_not_usage_bearing() {
    let (mirror, pool, _cfg) = setup("group").await;
    let (machine, session) = scaffold(&pool, "group").await;

    let first = insert_message(
        &pool,
        machine,
        session,
        Some("msg_shared"),
        "2026-07-01T10:00:00Z",
    )
    .await;
    mirror.refresh(&pool).await.expect("first refresh");

    let second = insert_message(
        &pool,
        machine,
        session,
        Some("msg_shared"),
        "2026-07-01T10:00:01Z",
    )
    .await;
    mirror.refresh(&pool).await.expect("second refresh");

    let rows = usage_rows(&mirror, session);
    assert_eq!(rows.len(), 2, "both rows mirrored");
    assert_eq!(
        rows.iter().find(|(id, _)| *id == first).map(|(_, u)| *u),
        Some(true),
        "the lowest id in the group carries usage"
    );
    assert_eq!(
        rows.iter().find(|(id, _)| *id == second).map(|(_, u)| *u),
        Some(false),
        "a later arrival in the same group must not also carry usage"
    );
}

/// The Time Machine backfill shape: an OLDER timestamp under a NEWER id. The
/// group minimum is by id, not by time, so the pre-existing row keeps usage.
#[tokio::test]
async fn backfilled_row_with_an_older_timestamp_does_not_steal_usage() {
    let (mirror, pool, _cfg) = setup("backfill").await;
    let (machine, session) = scaffold(&pool, "backfill").await;

    let live = insert_message(
        &pool,
        machine,
        session,
        Some("msg_bf"),
        "2026-07-01T10:00:00Z",
    )
    .await;
    mirror.refresh(&pool).await.expect("first refresh");

    // Same logical message, timestamp a year earlier, id necessarily higher.
    let backfilled = insert_message(
        &pool,
        machine,
        session,
        Some("msg_bf"),
        "2025-07-01T10:00:00Z",
    )
    .await;
    assert!(
        backfilled > live,
        "ids are monotonic regardless of timestamp"
    );
    mirror.refresh(&pool).await.expect("second refresh");

    let rows = usage_rows(&mirror, session);
    assert_eq!(
        rows.iter().find(|(id, _)| *id == live).map(|(_, u)| *u),
        Some(true),
        "the original row keeps usage — the group minimum is by id, not time"
    );
    assert_eq!(
        rows.iter()
            .find(|(id, _)| *id == backfilled)
            .map(|(_, u)| *u),
        Some(false),
        "an older-timestamped backfill must not become the usage row"
    );
}

/// The case a naive `WHERE id > max_id` gets wrong.
///
/// Simulates a transaction that committed late: a row whose id sits *below* the
/// mirror's high watermark appears only after the watermark has passed it. The
/// overlap re-scan must still collect it, exactly once.
#[tokio::test]
async fn a_row_appearing_below_the_watermark_is_still_collected() {
    let (mirror, pool, _cfg) = setup("overlap").await;
    let (machine, session) = scaffold(&pool, "overlap").await;

    // Two rows exist; only the *higher* one is visible to the first refresh,
    // which is what an uncommitted lower-id transaction looks like from here.
    let late = insert_message(
        &pool,
        machine,
        session,
        Some("msg_late"),
        "2026-07-01T10:00:00Z",
    )
    .await;
    let ahead = insert_message(
        &pool,
        machine,
        session,
        Some("msg_ahead"),
        "2026-07-01T10:00:02Z",
    )
    .await;
    sqlx::query("DELETE FROM messages WHERE id = $1")
        .bind(late)
        .execute(&pool)
        .await
        .expect("hide the late row");

    mirror.refresh(&pool).await.expect("refresh past the gap");
    assert_eq!(count_for(&mirror, session), 1, "only the higher row so far");
    match mirror.state() {
        MirrorState::Ready { max_id, .. } => {
            assert!(max_id >= ahead, "watermark advanced past the missing row");
        }
        MirrorState::Warming => panic!("mirror should be ready after a refresh"),
    }

    // The late transaction commits: same id, now below the watermark.
    sqlx::query(
        r#"INSERT INTO messages
             (id, machine_id, session_id, uuid, message_id, provider, role, "timestamp",
            input_tokens, content, message_key, raw)
           OVERRIDING SYSTEM VALUE
           VALUES ($1, $4, $2, $3, 'msg_late', 'claude', 'assistant',
                   '2026-07-01T10:00:00Z'::timestamptz, 10, '[]'::jsonb, $3::text, '{}'::jsonb)"#,
    )
    .bind(late)
    .bind(session)
    .bind(Uuid::new_v4())
    .bind(machine)
    .execute(&pool)
    .await
    .expect("late commit");

    mirror.refresh(&pool).await.expect("refresh with overlap");

    assert_eq!(
        count_for(&mirror, session),
        2,
        "the late row must be collected by the overlap re-scan, not skipped forever"
    );
    let rows = usage_rows(&mirror, session);
    assert!(
        rows.iter().all(|(_, u)| *u),
        "distinct logical messages each carry their own usage"
    );

    // And re-running must not duplicate anything.
    mirror.refresh(&pool).await.expect("idempotent re-refresh");
    assert_eq!(count_for(&mirror, session), 2, "refresh is idempotent");
}
