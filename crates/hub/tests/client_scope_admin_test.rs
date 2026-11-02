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

/// R2-F: only an entry whose own text and every session match nothing under
/// the full router (markers AND terms), with every session lab, is restamped.
/// An unstamped entry for the restamp tests: its id, project path and `generated_at`.
async fn unstamped_entry(pool: &PgPool, sessions: &[i64]) -> (i64, String, String) {
    let project = format!("/w/rs-{}", Uuid::new_v4());
    let (id, at): (i64, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "INSERT INTO journal_entries (entry_date, project_path, status, model, headline, summary, session_ids)
         VALUES ('2026-09-01', $1, 'entry', 'gpt-x', 'lab day', 's', $2) RETURNING id, generated_at",
    )
    .bind(&project)
    .bind(sessions)
    .fetch_one(pool)
    .await
    .unwrap();
    (
        id,
        project,
        at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
    )
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

/// E8 (design R3-E8): the hub applies only what the distiller's router listed,
/// and only when nothing changed since the router read it.
#[tokio::test]
async fn restamp_from_file_applies_only_unchanged_lab_entries() {
    let pool = pool().await;
    let (p, _dir) = policy(&routing("\"/clientco\""), 1);
    let g = p.generation.clone();
    sqlx::query("UPDATE client_scope_state SET generation = $1")
        .bind(&g)
        .execute(&pool)
        .await
        .unwrap();
    let lab = session(&pool, "plain lab work", Some("none"), Some(&g)).await;
    let lab2 = session(&pool, "more lab work", Some("none"), Some(&g)).await;
    let grows = session(&pool, "lab work", Some("none"), Some(&g)).await;
    let flagged = session(&pool, "x", Some("text_marker"), Some(&g)).await;
    let old_gen = session(&pool, "lab work", Some("none"), Some("old")).await;
    let started = chrono::Utc::now();
    std::thread::sleep(std::time::Duration::from_millis(20));

    let (clean, clean_p, clean_at) = unstamped_entry(&pool, &[lab]).await;
    let (_, regen_p, _) = unstamped_entry(&pool, &[lab]).await;
    let (_, moved_p, moved_at) = unstamped_entry(&pool, &[lab]).await;
    let (_, flag_p, flag_at) = unstamped_entry(&pool, &[flagged]).await;
    let (_, gen_p, gen_at) = unstamped_entry(&pool, &[old_gen]).await;
    let (_, grow_p, grow_at) = unstamped_entry(&pool, &[grows]).await;
    let (stamped, stamped_p, stamped_at) = unstamped_entry(&pool, &[lab]).await;
    sqlx::query(
        "UPDATE journal_entries SET route = 'protected', route_at = generated_at WHERE id = $1",
    )
    .bind(stamped)
    .execute(&pool)
    .await
    .unwrap();
    // `grows` gets a message after the router's read.
    sqlx::query(
        "INSERT INTO messages (session_id, machine_id, provider, message_key, content, raw)
         SELECT id, machine_id, 'claude', $2, '[]', '{}' FROM sessions WHERE id = $1",
    )
    .bind(grows)
    .bind(Uuid::new_v4().to_string())
    .execute(&pool)
    .await
    .unwrap();
    // Messages inserted above by `session()` predate `started`; these entries'
    // sessions did not change, but the file's view of them did:
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
            (stamped_p, stamped_at, vec![lab]),
            ("/w/rs-missing".into(), clean_at, vec![lab]),
        ],
    );

    // The classify run "finished" 5 s after it started; apply after that.
    let later = || chrono::Utc::now() + chrono::Duration::seconds(10);
    let dry = restamp_from_file(&pool, &file, None, later())
        .await
        .unwrap();
    assert_eq!(
        dry.render(),
        "listed 8, restamped 1; skipped: changed_since 1, gone 1, regenerated 1, \
                               session_not_lab 2, sessions_differ 1, stamped 1"
    );
    assert_eq!(
        route_of(&pool, clean).await,
        None,
        "a dry run writes nothing"
    );
    let before = audits(&pool, "restamp-history").await;

    let done = restamp_from_file(&pool, &file, Some("ac via the client desk"), later())
        .await
        .unwrap();
    assert_eq!(done.render(), dry.render());
    let (route, same): (Option<String>, bool) =
        sqlx::query_as("SELECT route, route_at = generated_at FROM journal_entries WHERE id = $1")
            .bind(clean)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((route.as_deref(), same), (Some("general-restamped"), true));
    assert_eq!(audits(&pool, "restamp-history").await, before + 1);
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

/// E8: the whole file is refused when it cannot be trusted, and E7: an empty
/// approver is refused.
#[tokio::test]
async fn restamp_from_file_refuses_an_untrusted_file() {
    let pool = pool().await;
    let (p, _dir) = policy(&routing("\"/clientco\""), 1);
    let g = p.generation.clone();
    sqlx::query("UPDATE client_scope_state SET generation = $1")
        .bind(&g)
        .execute(&pool)
        .await
        .unwrap();
    let lab = session(&pool, "plain lab work", Some("none"), Some(&g)).await;
    let started = chrono::Utc::now() - chrono::Duration::seconds(30);
    let (id, project, at) = unstamped_entry(&pool, &[lab]).await;
    let item = [(project, at, vec![lab])];
    let now = chrono::Utc::now();
    let ok = hub::client_admin::RESTAMP_MIN_DISTILLER;
    for (file, by, why) in [
        (
            restamp_file(&"b".repeat(64), ok, started, &item),
            None,
            "generation",
        ),
        (
            restamp_file(&g, "0.39.1", started, &item),
            None,
            "distiller",
        ),
        (
            restamp_file(&g, ok, started - chrono::Duration::hours(25), &item),
            None,
            "older than 24 h",
        ),
        (
            restamp_file(&g, ok, started, &item),
            Some(""),
            "approved-by",
        ),
        (
            restamp_file(&g, ok, started, &item),
            Some("  "),
            "approved-by",
        ),
        (b"not json".to_vec(), None, "not a restamp file"),
    ] {
        let err = restamp_from_file(&pool, &file, by, now).await.unwrap_err();
        assert!(err.to_string().contains(why), "{why}: {err}");
    }
    assert_eq!(route_of(&pool, id).await, None);
}
