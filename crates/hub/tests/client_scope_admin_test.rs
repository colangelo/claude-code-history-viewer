//! Audited client-scope actions (openspec `hub-client-read-scope`, task 2.5).
//!
//! Requires `TEST_DATABASE_URL`; run with `--test-threads=1`.

use hub::client_admin::{accept_routing, clear, restamp_history, unlatch};
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
#[tokio::test]
async fn restamp_history_needs_clean_text_lab_sessions_and_no_terms() {
    let pool = pool().await;
    let (p, _dir) = policy(&routing("\"/clientco\""), 1);
    let g = p.generation.clone();
    let lab = session(&pool, "plain lab work", Some("none"), Some(&g)).await;
    let term = session(&pool, "the clientword deal", Some("none"), Some(&g)).await;
    let client = session(&pool, "x", Some("text_marker"), Some(&g)).await;
    let entry = |headline: &str, sessions: Vec<i64>| {
        let pool = pool.clone();
        let headline = headline.to_string();
        async move {
            sqlx::query_scalar::<_, i64>(
                "INSERT INTO journal_entries (entry_date, project_path, status, model, headline, summary, session_ids)
                 VALUES ('2026-09-01', $1, 'entry', 'gpt-x', $2, 's', $3) RETURNING id",
            )
            .bind(format!("/w/rs-{}", Uuid::new_v4()))
            .bind(headline)
            .bind(sessions)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let clean = entry("lab day", vec![lab]).await;
    let text_hit = entry("work on /w/clientco today", vec![lab]).await;
    let term_hit = entry("lab day", vec![term]).await;
    let client_entry = entry("lab day", vec![client]).await;

    let dry = restamp_history(&pool, &p, None).await.unwrap();
    assert!(
        dry.restampable >= 1
            && dry.entry_text_hit >= 1
            && dry.session_term_hit >= 1
            && dry.session_not_lab >= 1
    );
    let route: Option<String> =
        sqlx::query_scalar("SELECT route FROM journal_entries WHERE id = $1")
            .bind(clean)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(route, None, "a dry run writes nothing");

    restamp_history(&pool, &p, Some("ac via the client desk"))
        .await
        .unwrap();
    let (route, same): (Option<String>, bool) =
        sqlx::query_as("SELECT route, route_at = generated_at FROM journal_entries WHERE id = $1")
            .bind(clean)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((route.as_deref(), same), (Some("general-restamped"), true));
    let others: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM journal_entries WHERE id IN ($1, $2, $3) AND route IS NOT NULL",
    )
    .bind(text_hit)
    .bind(term_hit)
    .bind(client_entry)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(others, 0, "the other three stay hidden");
}
