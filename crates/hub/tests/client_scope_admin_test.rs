//! Audited client-scope actions (openspec `hub-client-read-scope`, task 2.5).
//!
//! Requires `TEST_DATABASE_URL`; run with `--test-threads=1`.

use hub::client_admin::{accept_routing, clear, restamp_from_file, unlatch};
use hub::client_policy::{load_paths, Load, Policy};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    Sha256::digest(b).iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

fn routing(markers: &str) -> String {
    format!("[protected]\nmodel = \"claude-x\"\npath_markers = [{markers}]\ncontent_terms = [\"clientword\"]\n")
}

/// A policy from routing text, written to a fresh folder.
fn policy(text: &str, plain: usize) -> (Policy, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("cchv-cs-admin-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("r.toml"), text).unwrap();
    std::fs::write(
        dir.join("f.json"),
        json!({"sha256": hex(text.as_bytes()), "plain_path_markers": plain,
               "encoded_path_markers": 0, "content_terms": 1})
        .to_string(),
    )
    .unwrap();
    let Load::Ready(p) = load_paths(&dir.join("r.toml"), &dir.join("f.json")) else {
        panic!("policy")
    };
    (p, dir)
}

async fn pool() -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    hub::MIGRATOR.run(&pool).await.unwrap();
    sqlx::query("DELETE FROM client_scope_policy")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE client_scope_state SET generation = NULL, enforced_at = NULL")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

/// A session with one message, its class and generation set directly.
async fn session(pool: &PgPool, text: &str, class: Option<&str>, generation: Option<&str>) -> i64 {
    let machine = Uuid::new_v4();
    sqlx::query("INSERT INTO machines (machine_id, hostname, os) VALUES ($1, $2, 'macos')")
        .bind(machine)
        .bind(format!("ad-{}", &machine.simple().to_string()[..8]))
        .execute(pool)
        .await
        .unwrap();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO sessions (machine_id, provider, session_id, client_class, client_gen)
         VALUES ($1, 'claude', $2, $3, $4) RETURNING id",
    )
    .bind(machine)
    .bind(Uuid::new_v4().to_string())
    .bind(class)
    .bind(generation)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO messages (session_id, machine_id, provider, message_key, content, raw)
         VALUES ($1, $2, 'claude', $3, $4, '{}')",
    )
    .bind(id)
    .bind(machine)
    .bind(Uuid::new_v4().to_string())
    .bind(json!([{"type": "text", "text": text}]))
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn audits(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM client_scope_audit WHERE action = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn clear_narrows_one_client_session_and_audits_it() {
    let pool = pool().await;
    let client = session(&pool, "x", Some("text_marker"), Some("g")).await;
    let lab = session(&pool, "x", Some("none"), Some("g")).await;
    let before = audits(&pool, "clear").await;
    clear(&pool, client, "false positive, reviewed", "ac")
        .await
        .unwrap();
    let (class, generation): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT client_class, client_gen FROM sessions WHERE id = $1")
            .bind(client)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (class, generation),
        (None, None),
        "unscanned again: lab only after a sweep"
    );
    assert_eq!(audits(&pool, "clear").await, before + 1);
    assert!(
        clear(&pool, lab, "x", "ac").await.is_err(),
        "a lab session has nothing to clear"
    );
}

#[tokio::test]
async fn unlatch_lifts_the_latch_and_audits_it() {
    let pool = pool().await;
    sqlx::query("UPDATE client_scope_state SET enforced_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    unlatch(&pool, "emergency", "ac").await.unwrap();
    let latched: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT enforced_at FROM client_scope_state")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(latched.is_none());
    assert!(audits(&pool, "unlatch").await >= 1);
}

/// R2-D: a narrowed set is unknown to the running hub until accepted.
#[tokio::test]
async fn accept_routing_makes_a_narrowed_set_usable() {
    let pool = pool().await;
    let (wide, wide_dir) = policy(&routing("\"/clientco\", \"/other\""), 2);
    hub::client_state::record_set(&pool, &wide, "test")
        .await
        .unwrap();
    let narrow_text = routing("\"/clientco\"");
    let (narrow, dir) = policy(&narrow_text, 1);
    let scope = hub::client_state::ClientScope::new(
        toml::from_str(&format!(
            "routing_file = \"{}\"\nfloor_file = \"{}\"\n",
            dir.join("r.toml").display(),
            dir.join("f.json").display()
        ))
        .unwrap(),
    );
    scope.refresh(&pool).await.unwrap();
    assert!(scope.snapshot().generation().is_none(), "narrowed: unknown");
    accept_routing(&pool, &narrow, "ac").await.unwrap();
    let scope = hub::client_state::ClientScope::new(
        toml::from_str(&format!(
            "routing_file = \"{}\"\nfloor_file = \"{}\"\n",
            dir.join("r.toml").display(),
            dir.join("f.json").display()
        ))
        .unwrap(),
    );
    scope.refresh(&pool).await.unwrap();
    assert_eq!(
        scope.snapshot().generation(),
        Some(narrow.generation.as_str())
    );
    let _ = wide_dir;
}

/// An unstamped entry for the restamp tests: its id, project path and
/// `generated_at`. `model` and `headline` default to a lab GPT entry.
async fn entry_with(
    pool: &PgPool,
    sessions: &[i64],
    model: &str,
    headline: &str,
    project: Option<&str>,
    generated_at: Option<chrono::DateTime<chrono::Utc>>,
) -> (i64, String, String) {
    let project = project.map_or_else(|| format!("/w/rs-{}", Uuid::new_v4()), str::to_string);
    let (id, at): (i64, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "INSERT INTO journal_entries (entry_date, project_path, status, model, headline, summary,
                                      session_ids, generated_at)
         VALUES ('2026-09-01', $1, 'entry', $2, $3, 's', $4, coalesce($5, now()))
         RETURNING id, generated_at",
    )
    .bind(&project)
    .bind(model)
    .bind(headline)
    .bind(sessions)
    .bind(generated_at)
    .fetch_one(pool)
    .await
    .unwrap();
    (id, project, at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
}

async fn unstamped_entry(pool: &PgPool, sessions: &[i64]) -> (i64, String, String) {
    entry_with(pool, sessions, "gpt-x", "lab day", None, None).await
}

fn restamp_file(
    generation: &str,
    version: &str,
    started: chrono::DateTime<chrono::Utc>,
    items: &[(String, String, Vec<i64>)],
) -> Vec<u8> {
    json!({
        "format": 1,
        "distiller_version": version,
        "generation": generation,
        "started_at": started.to_rfc3339(),
        "finished_at": (started + chrono::Duration::seconds(5)).to_rfc3339(),
        "counts": {},
        "restampable": items.iter().map(|(project, at, sids)| json!({
            "entry_date": "2026-09-01", "project_path": project,
            "generated_at": at, "session_ids": sids,
        })).collect::<Vec<_>>(),
    })
    .to_string()
    .into_bytes()
}

async fn route_of(pool: &PgPool, id: i64) -> Option<String> {
    sqlx::query_scalar("SELECT route FROM journal_entries WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The hub CLI's dry-run pool: every transaction read-only (security E8 R1).
async fn read_only_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET default_transaction_read_only = on")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap()
}

async fn set_generation(pool: &PgPool, g: &str) {
    sqlx::query("UPDATE client_scope_state SET generation = $1")
        .bind(g)
        .execute(pool)
        .await
        .unwrap();
}

/// E8 (design R3-E8) and security's code review R1-R7: the hub applies only
/// what the distiller's router listed, re-checks the entry against the same
/// policy, and skips anything that changed since the router read it.
#[tokio::test]
async fn restamp_from_file_applies_only_unchanged_lab_entries() {
    let pool = pool().await;
    let (p, _dir) = policy(&routing("\"/clientco\""), 1);
    let g = p.generation.clone();
    set_generation(&pool, &g).await;
    let lab = session(&pool, "plain lab work", Some("none"), Some(&g)).await;
    let lab2 = session(&pool, "more lab work", Some("none"), Some(&g)).await;
    let grows = session(&pool, "lab work", Some("none"), Some(&g)).await;
    let row_only = session(&pool, "lab work", Some("none"), Some(&g)).await;
    let flagged = session(&pool, "x", Some("text_marker"), Some(&g)).await;
    let old_gen = session(&pool, "lab work", Some("none"), Some("old")).await;
    // The classify run started well after these sessions were written (the
    // margin covers one ingest batch).
    let margin = chrono::Duration::seconds(hub::client_admin::RESTAMP_INGEST_MARGIN_SECS);
    let started = chrono::Utc::now() + margin + chrono::Duration::seconds(60);
    let cutoff = chrono::DateTime::parse_from_rfc3339("2026-10-21T10:16:59Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let (clean, clean_p, clean_at) = unstamped_entry(&pool, &[lab]).await;
    let (_, regen_p, _) = unstamped_entry(&pool, &[lab]).await;
    let (_, moved_p, moved_at) = unstamped_entry(&pool, &[lab]).await;
    let (_, flag_p, flag_at) = unstamped_entry(&pool, &[flagged]).await;
    let (_, gen_p, gen_at) = unstamped_entry(&pool, &[old_gen]).await;
    let (_, grow_p, grow_at) = unstamped_entry(&pool, &[grows]).await;
    let (_, row_p, row_at) = unstamped_entry(&pool, &[row_only]).await;
    let (stamped, stamped_p, stamped_at) = unstamped_entry(&pool, &[lab]).await;
    let (stale, stale_p, stale_at) = unstamped_entry(&pool, &[lab]).await;
    let (_, text_p, text_at) = entry_with(&pool, &[lab], "gpt-x", "work on /w/clientco", None, None).await;
    let (_, term_p, term_at) = entry_with(&pool, &[lab], "gpt-x", "the clientword deal", None, None).await;
    let path = format!("/w/clientco/{}", Uuid::new_v4());
    let (_, path_p, path_at) = entry_with(&pool, &[lab], "gpt-x", "lab day", Some(&path), None).await;
    let (_, claude_p, claude_at) = entry_with(
        &pool, &[lab], "claude-haiku-5-5", "lab day", None,
        Some(cutoff + chrono::Duration::hours(1)),
    )
    .await;
    let (july, july_p, july_at) = entry_with(
        &pool, &[lab], "haiku", "lab day", None,
        Some(cutoff - chrono::Duration::days(90)),
    )
    .await;
    sqlx::query("UPDATE journal_entries SET route = 'protected', route_at = generated_at WHERE id = $1")
        .bind(stamped)
        .execute(&pool)
        .await
        .unwrap();
    // R3: a stale protected stamp is still a stamp.
    sqlx::query("UPDATE journal_entries SET route = 'protected', route_at = generated_at - interval '1 hour' WHERE id = $1")
        .bind(stale)
        .execute(&pool)
        .await
        .unwrap();
    // `grows` gets a message inside the margin before the run started.
    sqlx::query(
        "INSERT INTO messages (session_id, machine_id, provider, message_key, content, raw, created_at)
         SELECT id, machine_id, 'claude', $2, '[]', '{}', $3 FROM sessions WHERE id = $1",
    )
    .bind(grows)
    .bind(Uuid::new_v4().to_string())
    .bind(started - chrono::Duration::seconds(30))
    .execute(&pool)
    .await
    .unwrap();
    // `row_only`: only its session row changed (a new summary).
    sqlx::query("UPDATE sessions SET summary = 'new', updated_at = $2 WHERE id = $1")
        .bind(row_only)
        .bind(started)
        .execute(&pool)
        .await
        .unwrap();
    let file = restamp_file(
        &g,
        hub::client_admin::RESTAMP_MIN_DISTILLER,
        started,
        &[
            (clean_p, clean_at.clone(), vec![lab]),
            (regen_p, "2026-01-01T00:00:00.000000Z".into(), vec![lab]),
            (moved_p, moved_at, vec![lab2]),
            (flag_p, flag_at, vec![flagged]),
            (gen_p, gen_at, vec![old_gen]),
            (grow_p, grow_at, vec![grows]),
            (row_p, row_at, vec![row_only]),
            (stamped_p, stamped_at, vec![lab]),
            (stale_p, stale_at, vec![lab]),
            (text_p, text_at, vec![lab]),
            (term_p, term_at, vec![lab]),
            (path_p, path_at, vec![lab]),
            (claude_p, claude_at, vec![lab]),
            (july_p, july_at, vec![lab]),
            ("/w/rs-missing".into(), clean_at, vec![lab]),
        ],
    );
    let later = || started + chrono::Duration::seconds(10);
    let want = "listed 15, restamped 2; skipped: changed_since 2, entry_text 2, gone 1, path 1, \
                regenerated 1, session_not_lab 2, sessions_differ 1, stamped 2, was_protected 1";

    // R1: the CLI's dry run runs on a read-only pool.
    let dry = restamp_from_file(&read_only_pool().await, &p, &file, None, later())
        .await
        .unwrap();
    assert_eq!(dry.render(), want);
    assert_eq!(route_of(&pool, clean).await, None, "a dry run writes nothing");
    let before = audits(&pool, "restamp-history").await;

    let done = restamp_from_file(&pool, &p, &file, Some("ac via the client desk"), later())
        .await
        .unwrap();
    assert_eq!(done.render(), want);
    for id in [clean, july] {
        let (route, same): (Option<String>, bool) = sqlx::query_as(
            "SELECT route, route_at = generated_at FROM journal_entries WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((route.as_deref(), same), (Some("general-restamped"), true));
    }
    assert_eq!(audits(&pool, "restamp-history").await, before + 2);
    let (gen, detail): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT generation, detail FROM client_scope_audit WHERE action = 'restamp-history' AND entry_id = $1",
    )
    .bind(clean)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(gen.as_deref(), Some(g.as_str()));
    let detail = detail.unwrap();
    assert!(
        detail.contains(&hex(&file)) && detail.contains(hub::client_admin::RESTAMP_MIN_DISTILLER),
        "{detail}"
    );
}

/// E7 with E8: when the audit row cannot be written, the route write rolls back.
#[tokio::test]
async fn restamp_route_write_rolls_back_with_a_failed_audit() {
    let pool = pool().await;
    let (p, _dir) = policy(&routing("\"/clientco\""), 1);
    let g = p.generation.clone();
    set_generation(&pool, &g).await;
    let lab = session(&pool, "plain lab work", Some("none"), Some(&g)).await;
    let started = chrono::Utc::now()
        + chrono::Duration::seconds(hub::client_admin::RESTAMP_INGEST_MARGIN_SECS + 60);
    let (id, project, at) = unstamped_entry(&pool, &[lab]).await;
    sqlx::query(
        "CREATE OR REPLACE FUNCTION test_fail_audit() RETURNS trigger AS $$
         BEGIN IF NEW.approved_by = 'fail-the-audit' THEN RAISE EXCEPTION 'audit refused'; END IF;
         RETURN NEW; END $$ LANGUAGE plpgsql",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("DROP TRIGGER IF EXISTS test_fail_audit ON client_scope_audit")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER test_fail_audit BEFORE INSERT ON client_scope_audit
         FOR EACH ROW EXECUTE FUNCTION test_fail_audit()",
    )
    .execute(&pool)
    .await
    .unwrap();
    let file = restamp_file(&g, hub::client_admin::RESTAMP_MIN_DISTILLER, started, &[(project, at, vec![lab])]);
    let res = restamp_from_file(
        &pool, &p, &file, Some("fail-the-audit"),
        started + chrono::Duration::seconds(10),
    )
    .await;
    sqlx::query("DROP TRIGGER test_fail_audit ON client_scope_audit")
        .execute(&pool)
        .await
        .unwrap();
    assert!(res.is_err());
    assert_eq!(route_of(&pool, id).await, None);
}

/// E8: the whole file is refused when it cannot be trusted, and E7: an empty
/// approver is refused.
#[tokio::test]
async fn restamp_from_file_refuses_an_untrusted_file() {
    let pool = pool().await;
    let (p, _dir) = policy(&routing("\"/clientco\""), 1);
    let (other, _dir2) = policy(&routing("\"/clientco\", \"/otherco\""), 2);
    let g = p.generation.clone();
    set_generation(&pool, &g).await;
    let lab = session(&pool, "plain lab work", Some("none"), Some(&g)).await;
    let started = chrono::Utc::now() - chrono::Duration::seconds(30);
    let (id, project, at) = unstamped_entry(&pool, &[lab]).await;
    let item = [(project, at, vec![lab])];
    let now = chrono::Utc::now();
    let ok = hub::client_admin::RESTAMP_MIN_DISTILLER;
    for (pol, file, by, why) in [
        (&p, restamp_file(&"b".repeat(64), ok, started, &item), None, "generation"),
        (&other, restamp_file(&g, ok, started, &item), None, "routing files"),
        (&p, restamp_file(&g, "0.39.1", started, &item), None, "distiller"),
        (&p, restamp_file(&g, ok, started - chrono::Duration::hours(25), &item), None, "older than 24 h"),
        (&p, restamp_file(&g, ok, started, &item), Some(""), "approved-by"),
        (&p, restamp_file(&g, ok, started, &item), Some("  "), "approved-by"),
        (&p, b"not json".to_vec(), None, "not a restamp file"),
    ] {
        let err = restamp_from_file(&pool, pol, &file, by, now).await.unwrap_err();
        assert!(err.to_string().contains(why), "{why}: {err}");
    }
    // R2: the accepted generation moved after the file was written.
    set_generation(&pool, &"c".repeat(64)).await;
    let err = restamp_from_file(&pool, &p, &restamp_file(&g, ok, started, &item), None, now)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("generation"), "{err}");
    assert_eq!(route_of(&pool, id).await, None);
}

/// Security E8 P3: the apply reads only a private regular file.
#[test]
fn restamp_file_must_be_private_and_not_a_link() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("cchv-rs-file-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.json");
    std::fs::write(&good, b"{}").unwrap();
    std::fs::set_permissions(&good, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(hub::client_admin::read_restamp_file(&good).unwrap(), b"{}");
    let open = dir.join("open.json");
    std::fs::write(&open, b"{}").unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(hub::client_admin::read_restamp_file(&open).unwrap_err().to_string().contains("0600"));
    let link = dir.join("link.json");
    std::os::unix::fs::symlink(&good, &link).unwrap();
    assert!(hub::client_admin::read_restamp_file(&link).is_err());
}
