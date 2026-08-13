//! Mirror lifecycle: refresher resilience and rebuild — change
//! `hub-stats-duckdb-mirror`, tasks 2.8 and 3.3.
//!
//! Two properties that only show up under failure, and would otherwise be found
//! in production:
//!
//! * a refresh that cannot reach Postgres must leave the mirror **intact and
//!   still serving**, and must not take the process down (design D6);
//! * `hub mirror rebuild` must correct a mirror that a Postgres-side `UPDATE`
//!   has invalidated, and the running hub must **notice the swapped file** —
//!   otherwise the rebuild fixes a file nobody is reading (design D2).
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use hub::config::MirrorConfig;
use hub::mirror::{self, Mirror, MirrorState, RefreshOutcome};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
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

fn cfg(tag: &str) -> MirrorConfig {
    let dir = std::env::temp_dir().join(format!("cchv-life-{tag}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    MirrorConfig {
        path: Some(dir.join("stats.duckdb")),
        ..MirrorConfig::default()
    }
}

/// A machine/project/session scaffold plus one message, returning the ids.
async fn seed(pool: &PgPool, label: &str) -> (Uuid, i64, i64) {
    let machine_id = Uuid::new_v4();
    sqlx::query("INSERT INTO machines (machine_id, hostname) VALUES ($1, $2)")
        .bind(machine_id)
        .bind(format!("host-{label}"))
        .execute(pool)
        .await
        .expect("machine");
    let project_id: i64 = sqlx::query_scalar(
        "INSERT INTO projects (machine_id, provider, project_path, name)
         VALUES ($1, 'claude', $2, $3) RETURNING id",
    )
    .bind(machine_id)
    .bind(format!("/tmp/{label}-{machine_id}"))
    .bind(label)
    .fetch_one(pool)
    .await
    .expect("project");
    let session_id: i64 = sqlx::query_scalar(
        "INSERT INTO sessions (machine_id, provider, session_id, project_id)
         VALUES ($1, 'claude', $2, $3) RETURNING id",
    )
    .bind(machine_id)
    .bind(Uuid::new_v4().to_string())
    .bind(project_id)
    .fetch_one(pool)
    .await
    .expect("session");
    (machine_id, project_id, session_id)
}

async fn add_message(pool: &PgPool, machine: Uuid, session: i64, key: &str, tokens: i64) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO messages
             (machine_id, session_id, uuid, provider, role, "timestamp",
              input_tokens, content, message_key, raw)
           VALUES ($1, $2, $3, 'claude', 'assistant', '2026-07-20T10:00:00Z'::timestamptz,
                   $5, '[]'::jsonb, $4, '{}'::jsonb)
           RETURNING id"#,
    )
    .bind(machine)
    .bind(session)
    .bind(Uuid::new_v4())
    .bind(key)
    .bind(tokens)
    .fetch_one(pool)
    .await
    .expect("message")
}

fn usage_tokens(mirror: &Mirror, session: i64) -> i64 {
    let conn = mirror.connection().expect("connection");
    conn.query_row(
        "SELECT coalesce(sum(input_tokens) FILTER (WHERE usage_row), 0)::BIGINT
           FROM messages WHERE session_id = ?",
        [session],
        |r| r.get(0),
    )
    .expect("sum")
}

/// Task 3.3. A refresher whose database is unreachable must keep running and
/// keep the existing mirror answerable — a stale answer beats no answer, and
/// the staleness is what `/v1/healthz/stats` reports.
#[tokio::test]
async fn repeated_refresh_failures_leave_the_mirror_serving() {
    let good = pool().await;
    let (machine, _project, session) = seed(&good, "resilient").await;
    add_message(&good, machine, session, "m1", 42).await;

    let mirror = Arc::new(Mirror::open_or_create(&cfg("resilient")).expect("open"));
    mirror.refresh(&good).await.expect("initial refresh");
    let before = mirror.state();
    assert!(before.is_ready(), "mirror is ready after the first refresh");
    assert_eq!(usage_tokens(&mirror, session), 42);

    // A pool that cannot connect: every refresh from here on fails.
    let dead = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(200))
        .connect_lazy("postgres://nobody@127.0.0.1:1/nothing")
        .expect("lazy pool");

    for attempt in 1..=3 {
        let err = mirror.refresh(&dead).await;
        assert!(
            err.is_err(),
            "refresh {attempt} should fail against a dead db"
        );
        assert!(
            mirror.state().is_ready(),
            "the mirror must stay ready through refresh failures"
        );
        assert_eq!(
            usage_tokens(&mirror, session),
            42,
            "a failed refresh must not disturb the data already mirrored"
        );
    }

    // And the long-running task must survive them rather than returning.
    let refresher = tokio::spawn(mirror::run_refresher(
        mirror.clone(),
        dead.clone(),
        Duration::from_millis(50),
    ));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !refresher.is_finished(),
        "the refresher exited on failure — it must never take the process with it"
    );
    refresher.abort();

    // Recovery needs no restart: point it back at a live database and it heals.
    add_message(&good, machine, session, "m2", 58).await;
    mirror.refresh(&good).await.expect("refresh after recovery");
    assert_eq!(usage_tokens(&mirror, session), 100);
}

/// A socket that accepts and then never speaks: TCP connects, so nothing
/// fails, but no byte of the Postgres handshake ever arrives. This is the
/// production shape (a connection `ESTABLISHED` on the client with no live
/// backend behind it) reduced to something deterministic and local — the
/// distinction that matters is *silence*, not refusal, and a refused
/// connection would exercise the already-covered `Err` path instead.
async fn blackhole_pool() -> PgPool {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind blackhole");
    let addr = listener.local_addr().expect("blackhole addr");
    tokio::spawn(async move {
        let mut accepted = Vec::new();
        while let Ok((sock, _)) = listener.accept().await {
            accepted.push(sock); // held open, never written to, never closed
        }
    });
    PgPoolOptions::new()
        .max_connections(1)
        // Far above the test's refresh budget: the point is to prove the
        // *refresh* ceiling fires, not sqlx's acquire ceiling.
        .acquire_timeout(Duration::from_secs(60))
        .connect_lazy(&format!("postgres://nobody@{addr}/nothing"))
        .expect("lazy pool")
}

/// A refresh that stops returning must be cancelled and must free the
/// single-flight latch — otherwise the *next* tick is skipped forever.
///
/// This is the regression test for the 2026-08-13 production wedge (infra
/// ac/infra#93). Both halves are load-bearing and the second is the one that
/// bites: a timeout that unblocked the loop but leaked the latch would leave
/// every later tick returning `Skipped`, which looks healthy in the logs and
/// leaves the mirror just as frozen. Needs no Postgres.
#[tokio::test]
async fn a_refresh_that_hangs_is_cancelled_and_does_not_wedge_later_ticks() {
    let dark = blackhole_pool().await;
    // The mirror is empty here, so `run_refresher` below charges this attempt
    // the *cold build* ceiling — left at its 6 h default the loop would park on
    // the first hang and the last assertion would pass for the wrong reason.
    let mirror = Arc::new(
        Mirror::open_or_create(&MirrorConfig {
            refresh_timeout_secs: 1,
            cold_build_timeout_secs: 1,
            ..cfg("hang")
        })
        .expect("open"),
    );

    let budget = Duration::from_millis(300);
    let started = std::time::Instant::now();
    let first = mirror
        .refresh_bounded(&dark, budget)
        .await
        .expect("a cancelled refresh is not an error");
    assert!(
        matches!(first, RefreshOutcome::TimedOut),
        "a refresh against a silent socket must time out, got {first:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "refresh_bounded returned only after sqlx's own timeout — the budget did not fire"
    );

    // The latch must have been released by the cancellation. If it leaked,
    // this second attempt returns `Skipped` and the refresher is wedged for
    // the life of the process.
    let second = mirror
        .refresh_bounded(&dark, budget)
        .await
        .expect("second attempt is not an error");
    assert!(
        matches!(second, RefreshOutcome::TimedOut),
        "the single-flight latch leaked on timeout — later ticks are wedged, got {second:?}"
    );

    // And the loop itself survives a hang rather than parking on it: with the
    // 1 s ceiling configured above, this window covers more than one full
    // cancel-and-retry cycle.
    let refresher = tokio::spawn(mirror::run_refresher(
        mirror.clone(),
        dark.clone(),
        Duration::from_millis(50),
    ));
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(
        !refresher.is_finished(),
        "the refresher exited on a hung refresh — it must never take the process with it"
    );
    refresher.abort();
}

/// Task 2.5. A tick arriving while a refresh is in flight is dropped, not
/// queued: queued ticks would only re-read rows the running one is already
/// collecting, and on a cold build they would stack several deep.
#[tokio::test]
async fn a_refresh_arriving_during_another_is_skipped() {
    let pool = pool().await;
    let (machine, _p, session) = seed(&pool, "singleflight").await;
    add_message(&pool, machine, session, "m1", 7).await;

    let mirror = Arc::new(Mirror::open_or_create(&cfg("singleflight")).expect("open"));
    let a = mirror.clone();
    let b = mirror.clone();
    let (pa, pb) = (pool.clone(), pool.clone());
    let (ra, rb) = tokio::join!(
        tokio::spawn(async move { a.refresh(&pa).await }),
        tokio::spawn(async move { b.refresh(&pb).await })
    );
    let outcomes = [
        ra.expect("join a").expect("refresh a"),
        rb.expect("join b").expect("refresh b"),
    ];
    let skipped = outcomes
        .iter()
        .filter(|o| matches!(o, RefreshOutcome::Skipped))
        .count();
    let ran = outcomes
        .iter()
        .filter(|o| matches!(o, RefreshOutcome::Ran(_)))
        .count();
    assert_eq!(ran + skipped, 2);
    assert!(ran >= 1, "at least one refresh must actually run");
    // Whichever way the race resolves, the mirror is correct afterwards.
    assert_eq!(usage_tokens(&mirror, session), 7);
}

/// Task 2.8. The case incremental refresh is explicitly not responsible for: a
/// Postgres-side `UPDATE` that regroups already-mirrored rows. Before the
/// rebuild the mirror over-counts; after it, it does not — and the *running*
/// mirror picks up the swapped file without a restart.
#[tokio::test]
async fn rebuild_corrects_a_regrouping_update_and_the_running_mirror_adopts_it() {
    let pool = pool().await;
    let (machine, _p, session) = seed(&pool, "rebuild").await;
    // Two rows that Postgres does not yet know are the same logical message.
    add_message(&pool, machine, session, "r1", 100).await;
    add_message(&pool, machine, session, "r2", 100).await;

    let config = cfg("rebuild");
    let mirror = Mirror::open_or_create(&config).expect("open");
    mirror.refresh(&pool).await.expect("refresh");
    assert_eq!(
        usage_tokens(&mirror, session),
        200,
        "two independent groups, so both carry usage"
    );

    // What `hub backfill-analytics` does: give existing rows a shared
    // provider message id. Postgres now sees one logical message; the mirror
    // still holds two, and therefore over-counts.
    sqlx::query("UPDATE messages SET message_id = 'msg_merged' WHERE session_id = $1")
        .bind(session)
        .execute(&pool)
        .await
        .expect("regroup");
    mirror
        .refresh(&pool)
        .await
        .expect("incremental refresh after the update");
    assert_eq!(
        usage_tokens(&mirror, session),
        200,
        "incremental refresh does NOT chase updates — this is the bug rebuild exists for"
    );

    let report = mirror::rebuild(&config, &pool).await.expect("rebuild");
    assert!(report.messages_inserted >= 2);
    assert!(report.cold_build, "a rebuild always starts from empty");

    // The still-open handle points at the replaced inode until it adopts.
    assert_eq!(
        usage_tokens(&mirror, session),
        200,
        "the old file is still being served, which is what keeps stats up during a rebuild"
    );
    assert!(
        mirror.adopt_replacement().expect("adopt"),
        "the swapped file must be detected"
    );
    assert_eq!(
        usage_tokens(&mirror, session),
        100,
        "after adopting the rebuilt mirror the merged group counts its usage once"
    );
    assert!(
        !mirror.adopt_replacement().expect("adopt again"),
        "adopting is idempotent — an unchanged file is not reopened every tick"
    );
    assert!(matches!(mirror.state(), MirrorState::Ready { .. }));
}

/// A mirror file that is not a database at all is moved aside and replaced,
/// never deleted — the same rule the binary-swap playbook uses, so a bad state
/// can still be inspected after the service has recovered.
#[test]
fn a_corrupt_mirror_is_moved_aside_and_replaced() {
    let config = cfg("corrupt");
    let path = config.resolved_path();
    std::fs::create_dir_all(path.parent().unwrap()).expect("dir");
    std::fs::write(&path, b"this is not a duckdb file").expect("write junk");

    let mirror = Mirror::open_or_create(&config).expect("open over a corrupt file");
    assert!(matches!(mirror.state(), MirrorState::Warming));

    let aside: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .expect("read dir")
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".aside-"))
        .collect();
    assert_eq!(aside.len(), 1, "the corrupt file is preserved, not deleted");
    assert_eq!(
        std::fs::read(aside[0].path()).expect("read aside"),
        b"this is not a duckdb file",
        "and preserved verbatim, so it can actually be diagnosed"
    );
}
