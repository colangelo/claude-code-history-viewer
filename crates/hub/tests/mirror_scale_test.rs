//! The cold build must not scale its memory with the row count.
//!
//! This guards a bug that shipped in the first cut of the refresher and that no
//! other test could see: every mirrored table was inserted one `INSERT`
//! statement per row, and `DuckDB` pins a 256 KiB block per statement for as
//! long as the enclosing transaction is open. Cost, measured: **~370 KiB of
//! memory per row** — 676 rows exhausted a 256 MB limit and 2,753 rows exhausted
//! the 1 GB default. A cold build of the real archive (2.8M messages) would
//! therefore have died a few thousand rows in, every time, and since a mirror
//! that never finishes never starts serving, `/v1/stats/*` would have answered
//! `503 warming` forever after the deploy.
//!
//! It was invisible because correctness tests seed a handful of rows: the
//! per-row cost is only fatal in bulk. So the test is written the only way it
//! can bite — a **tight memory limit** and enough rows that the old path could
//! not have passed.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use hub::config::MirrorConfig;
use hub::mirror::Mirror;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests")
}

/// Rows to seed. At the old per-row cost this is ~740 MB against the 256 MB
/// limit below — roughly 3x over, so the test fails on the old path by a margin
/// rather than by a hair. Kept small because CI runs it on every push; raise it
/// with `CCHV_SCALE_ROWS` to rehearse a bigger build by hand (200,000 was run
/// locally when the batching landed, and stayed flat).
const ROWS: i64 = 2_000;

fn rows() -> i64 {
    std::env::var("CCHV_SCALE_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(ROWS)
}

/// Tight on purpose. The production default is 1 GB; a limit that generous would
/// let ~2,700 per-row inserts through and the test would pass on the very code
/// it exists to reject.
const MEMORY_LIMIT: &str = "256MB";

#[tokio::test]
async fn a_cold_build_of_many_rows_fits_in_a_tight_memory_limit() {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&test_db_url())
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");

    let rows = rows();
    let (machine, session) = seed_bulk(&pool, rows).await;

    let dir = std::env::temp_dir().join(format!("cchv-scale-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mirror = Mirror::open_or_create(&MirrorConfig {
        path: Some(dir.join("stats.duckdb")),
        memory_limit: MEMORY_LIMIT.to_string(),
        ..MirrorConfig::default()
    })
    .expect("open mirror");

    // The assertion is that this returns at all: on the old path it fails with
    // `Out of Memory Error: failed to pin block of size 256.0 KiB`.
    let report = mirror
        .refresh(&pool)
        .await
        .expect("cold build must fit in the memory limit")
        .report()
        .copied()
        .expect("the first refresh runs rather than skipping");
    assert!(report.cold_build, "an empty mirror builds cold");
    println!(
        "cold build: {} messages mirrored in {:.1}s, file {:.1} MiB",
        report.messages_inserted,
        report.elapsed.as_secs_f64(),
        file_mib(mirror.path()),
    );

    // Same batching change could have dropped or misplaced rows, which a
    // memory-only assertion would not notice. So: every seeded row arrives, and
    // its columns land where they belong.
    let conn = mirror.connection().expect("mirror connection");
    let (count, tokens, conversational, usage): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT count(*)::BIGINT,
                    coalesce(sum(input_tokens), 0)::BIGINT,
                    count(*) FILTER (WHERE conversational)::BIGINT,
                    count(*) FILTER (WHERE usage_row)::BIGINT
               FROM messages WHERE session_id = ?",
            [session],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .expect("read back the mirrored rows");
    assert_eq!(count, rows, "every seeded row must be mirrored");
    assert_eq!(tokens, rows, "one input token per row, in the right column");
    assert_eq!(conversational, rows, "all seeded rows carry a role");
    assert_eq!(
        usage, rows,
        "distinct uuids mean distinct dedup groups, so all rows are usage-bearing"
    );

    // The mirror is derived state, but the *archive* is not a scratch pad: the
    // test database persists between runs, so 2,000 rows per run would
    // accumulate into every other test's global-scope comparison.
    cleanup(&pool, machine).await;
}

/// Insert `rows` messages in one statement, on their own machine so the cleanup
/// at the end can be scoped exactly.
async fn seed_bulk(pool: &PgPool, rows: i64) -> (Uuid, i64) {
    let machine = Uuid::new_v4();
    sqlx::query("INSERT INTO machines (machine_id, hostname) VALUES ($1, $2)")
        .bind(machine)
        .bind(format!("scale-{machine}"))
        .execute(pool)
        .await
        .expect("machine");
    let project: i64 = sqlx::query_scalar(
        "INSERT INTO projects (machine_id, provider, project_path, name)
         VALUES ($1, 'claude', $2, 'scale') RETURNING id",
    )
    .bind(machine)
    .bind(format!("/tmp/scale-{machine}"))
    .fetch_one(pool)
    .await
    .expect("project");
    let session: i64 = sqlx::query_scalar(
        "INSERT INTO sessions (machine_id, provider, session_id, project_id)
         VALUES ($1, $2, 'scale-session', $3) RETURNING id",
    )
    .bind(machine)
    .bind(format!("scale-{machine}"))
    .bind(project)
    .fetch_one(pool)
    .await
    .expect("session");

    sqlx::query(
        r#"INSERT INTO messages
             (machine_id, session_id, uuid, provider, role, "timestamp",
              input_tokens, content, message_key, raw)
           SELECT $1, $2, gen_random_uuid(), 'claude', 'assistant',
                  '2026-07-20T10:00:00Z'::timestamptz + make_interval(secs => g),
                  1, '[]'::jsonb, 'scale-' || g, '{}'::jsonb
             FROM generate_series(1, $3) g"#,
    )
    .bind(machine)
    .bind(session)
    .bind(rows)
    .execute(pool)
    .await
    .expect("bulk seed");

    (machine, session)
}

/// Size of the built mirror, so a scaled-up local run reports bytes per row —
/// the number the change's disk-headroom estimate needs (task 6.5).
fn file_mib(path: &std::path::Path) -> f64 {
    std::fs::metadata(path).map_or(0.0, |m| m.len() as f64 / (1024.0 * 1024.0))
}

async fn cleanup(pool: &PgPool, machine: Uuid) {
    for stmt in [
        "DELETE FROM messages WHERE machine_id = $1",
        "DELETE FROM sessions WHERE machine_id = $1",
        "DELETE FROM projects WHERE machine_id = $1",
        "DELETE FROM machines WHERE machine_id = $1",
    ] {
        sqlx::query(stmt)
            .bind(machine)
            .execute(pool)
            .await
            .expect("cleanup");
    }
}
