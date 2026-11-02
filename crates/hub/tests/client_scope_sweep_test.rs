//! The client-scope sweep (openspec `hub-client-read-scope`, R2-C, task 2.4).
//!
//! Requires `TEST_DATABASE_URL`; run with `--test-threads=1`.

use hub::client_sweep::{stamp, sweep, SweepOptions};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

const ROUTING: &str = r#"
[protected]
model = "claude-x"
path_markers = ["/clientco"]
content_terms = ["clientword"]
"#;

fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    Sha256::digest(b).iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

struct Env {
    pool: PgPool,
    policy: hub::client_policy::Policy,
    machine: Uuid,
    project: i64,
    _dir: std::path::PathBuf,
}

async fn env() -> Env {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    hub::MIGRATOR.run(&pool).await.unwrap();
    let dir = std::env::temp_dir().join(format!("cchv-cs-sweep-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("r.toml"), ROUTING).unwrap();
    std::fs::write(
        dir.join("f.json"),
        json!({"sha256": hex(ROUTING.as_bytes()), "plain_path_markers": 1,
               "encoded_path_markers": 0, "content_terms": 1})
        .to_string(),
    )
    .unwrap();
    let hub::client_policy::Load::Ready(policy) =
        hub::client_policy::load_paths(&dir.join("r.toml"), &dir.join("f.json"))
    else {
        panic!("policy")
    };
    let machine = Uuid::new_v4();
    sqlx::query("INSERT INTO machines (machine_id, hostname, os) VALUES ($1, $2, 'macos')")
        .bind(machine)
        .bind(format!("sw-{}", &machine.simple().to_string()[..8]))
        .execute(&pool)
        .await
        .unwrap();
    let project: i64 = sqlx::query_scalar(
        "INSERT INTO projects (machine_id, provider, project_path, name) VALUES ($1, 'claude', '/w/lab', 'p') RETURNING id",
    )
    .bind(machine)
    .fetch_one(&pool)
    .await
    .unwrap();
    Env {
        pool,
        policy,
        machine,
        project,
        _dir: dir,
    }
}

/// An unscanned session (as if it predates client scope) with these texts.
async fn session(e: &Env, texts: &[&str]) -> i64 {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO sessions (machine_id, provider, session_id, project_id) VALUES ($1, 'claude', $2, $3) RETURNING id",
    )
    .bind(e.machine)
    .bind(Uuid::new_v4().to_string())
    .bind(e.project)
    .fetch_one(&e.pool)
    .await
    .unwrap();
    for (i, t) in texts.iter().enumerate() {
        let content = json!([{"type": "text", "text": t}]);
        sqlx::query(
            "INSERT INTO messages (session_id, machine_id, provider, message_key, seq, content, raw, search_text)
             VALUES ($1, $6, 'claude', $2, $3, $4, $5, '')",
        )
        .bind(id)
        .bind(Uuid::new_v4().to_string())
        .bind(i32::try_from(i).unwrap())
        .bind(&content)
        .bind(json!({"cwd": "/w/lab", "message": {"content": content}}))
        .bind(e.machine)
        .execute(&e.pool)
        .await
        .unwrap();
    }
    id
}

async fn class_and_gen(e: &Env, id: i64) -> (Option<String>, Option<String>) {
    sqlx::query_as("SELECT client_class, client_gen FROM sessions WHERE id = $1")
        .bind(id)
        .fetch_one(&e.pool)
        .await
        .unwrap()
}

fn only(ids: &[i64]) -> SweepOptions {
    SweepOptions {
        dry_run: false,
        batch: 10,
        only_sessions: Some(ids.to_vec()),
        rescan: false,
    }
}

#[tokio::test]
async fn the_sweep_classifies_and_stamps_unscanned_sessions() {
    let e = env().await;
    let lab = session(&e, &["plain lab work"]).await;
    let client = session(
        &e,
        &["a", &format!("{} then clientco.", "x".repeat(70_000))],
    )
    .await;
    let report = sweep(&e.pool, &e.policy, &only(&[lab, client]))
        .await
        .unwrap();
    assert_eq!(report.scanned, 2);
    assert_eq!(
        class_and_gen(&e, lab).await,
        (Some("none".into()), Some(e.policy.generation.clone()))
    );
    assert_eq!(
        class_and_gen(&e, client).await,
        (
            Some("text_marker".into()),
            Some(e.policy.generation.clone())
        )
    );
}

#[tokio::test]
async fn a_dry_run_counts_and_writes_nothing() {
    let e = env().await;
    let lab = session(&e, &["lab"]).await;
    let client = session(&e, &["see clientco."]).await;
    let term = session(&e, &["the clientword thing"]).await;
    let report = sweep(
        &e.pool,
        &e.policy,
        &SweepOptions {
            dry_run: true,
            batch: 10,
            only_sessions: Some(vec![lab, client, term]),
            rescan: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(report.by_class.get("none"), Some(&2));
    assert_eq!(report.by_class.get("text_marker"), Some(&1));
    assert_eq!(
        report.term_only, 1,
        "a session the distiller's terms protect but the hub leaves lab"
    );
    for id in [lab, client, term] {
        assert_eq!(
            class_and_gen(&e, id).await,
            (None, None),
            "dry run wrote nothing"
        );
    }
}

/// An ingest between the sweep's read and its write leaves the session
/// unscanned: the stamp is conditional on the revision it read.
#[tokio::test]
async fn a_stale_revision_does_not_stamp() {
    let e = env().await;
    let id = session(&e, &["lab"]).await;
    sqlx::query("UPDATE sessions SET client_rev = 5 WHERE id = $1")
        .bind(id)
        .execute(&e.pool)
        .await
        .unwrap();
    stamp(&e.pool, id, "none", &e.policy.generation, 4)
        .await
        .unwrap();
    assert_eq!(
        class_and_gen(&e, id).await,
        (Some("none".into()), None),
        "class set, not stamped"
    );
    // A hit still raises the class even with a stale revision.
    stamp(&e.pool, id, "text_marker", &e.policy.generation, 4)
        .await
        .unwrap();
    assert_eq!(
        class_and_gen(&e, id).await.0.as_deref(),
        Some("text_marker")
    );
    // And nothing lowers it.
    stamp(&e.pool, id, "none", &e.policy.generation, 5)
        .await
        .unwrap();
    assert_eq!(
        class_and_gen(&e, id).await,
        (
            Some("text_marker".into()),
            Some(e.policy.generation.clone())
        )
    );
}

/// R2-B: the sessions of a protected journal entry are one family.
#[tokio::test]
async fn sessions_of_a_protected_entry_turn_client() {
    let e = env().await;
    let a = session(&e, &["lab"]).await;
    let b = session(&e, &["lab"]).await;
    sqlx::query(
        "INSERT INTO journal_entries (entry_date, project_path, status, model, headline, summary, session_ids, route, route_at, generated_at)
         VALUES ('2026-10-30', $1, 'entry', 'claude-x', 'h', 's', $2, 'protected', now(), now())",
    )
    .bind(format!("/w/sweep-{}", Uuid::new_v4()))
    .bind(vec![a, b])
    .execute(&e.pool)
    .await
    .unwrap();
    let report = sweep(&e.pool, &e.policy, &only(&[a, b])).await.unwrap();
    assert_eq!(report.family_protected_entry, 2);
    assert_eq!(
        class_and_gen(&e, a).await.0.as_deref(),
        Some("family:protected-entry")
    );
}

/// Security O3: the dry run's census and the real run's final classes agree:
/// copies on two machines, a parent and its subagent child, the sessions of a
/// protected entry, and a lab session the terms would protect.
#[tokio::test]
async fn the_census_matches_what_the_sweep_writes() {
    let e = env().await;
    let s_local = session(&e, &["see clientco."]).await; // local
    let s_copy = session(&e, &["lab"]).await; // copy of s_local, on another machine
    let other = Uuid::new_v4();
    sqlx::query("INSERT INTO machines (machine_id, hostname, os) VALUES ($1, $2, 'macos')")
        .bind(other)
        .bind(format!("sw2-{}", &other.simple().to_string()[..8]))
        .execute(&e.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE sessions SET machine_id = $1, session_id = (SELECT session_id FROM sessions WHERE id = $2) WHERE id = $3",
    )
    .bind(other)
    .bind(s_local)
    .bind(s_copy)
    .execute(&e.pool)
    .await
    .unwrap();
    let s_parent = session(&e, &["lab"]).await; // parent
    let s_child = session(&e, &["built clientco."]).await; // subagent child, local
    sqlx::query(
        "UPDATE sessions SET file_path = '/x/' || (SELECT session_id FROM sessions WHERE id = $1) || '/subagents/a.jsonl' WHERE id = $2",
    )
    .bind(s_parent)
    .bind(s_child)
    .execute(&e.pool)
    .await
    .unwrap();
    let e1 = session(&e, &["lab"]).await;
    let e2 = session(&e, &["lab"]).await;
    sqlx::query(
        "INSERT INTO journal_entries (entry_date, project_path, status, model, headline, summary, session_ids, route, route_at, generated_at)
         VALUES ('2026-10-30', $1, 'entry', 'claude-x', 'h', 's', $2, 'protected', now(), now())",
    )
    .bind(format!("/w/census-{}", Uuid::new_v4()))
    .bind(vec![e1, e2])
    .execute(&e.pool)
    .await
    .unwrap();
    let s_term = session(&e, &["the clientword deal"]).await;
    let s_lab = session(&e, &["plain lab"]).await;
    let ids = vec![s_local, s_copy, s_parent, s_child, e1, e2, s_term, s_lab];

    let census = hub::client_sweep::census(&e.pool, &e.policy, Some(&ids))
        .await
        .unwrap();
    assert_eq!(
        census,
        hub::client_sweep::Census {
            sessions: 8,
            local: 2,
            family_only: 2,
            protected_entry_only: 2,
            lab: 2,
            term_only: 1
        }
    );
    sweep(&e.pool, &e.policy, &only(&ids)).await.unwrap();
    let classes: Vec<Option<String>> =
        sqlx::query_scalar("SELECT client_class FROM sessions WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_all(&e.pool)
            .await
            .unwrap();
    let count =
        |f: &dyn Fn(&str) -> bool| classes.iter().flatten().filter(|cls| f(cls)).count() as u64;
    assert_eq!(count(&|cls| cls == "none"), census.lab);
    assert_eq!(
        count(&|cls| cls != "none" && !cls.starts_with("family:")),
        census.local
    );
    assert_eq!(
        count(&|cls| cls.starts_with("family:")),
        census.family_only + census.protected_entry_only
    );
}

/// E1: the project's name, git remote and main checkout path are classified
/// (as `path`), and a stored `search_text` is read like the text it indexes.
#[tokio::test]
async fn the_sweep_reads_project_fields_and_search_text() {
    let e = env().await;
    let lab = session(&e, &["plain lab work"]).await;
    let by_search_text = session(&e, &["plain lab work"]).await;
    sqlx::query("UPDATE messages SET search_text = 'notes in /w/clientco/x' WHERE session_id = $1")
        .bind(by_search_text)
        .execute(&e.pool)
        .await
        .unwrap();
    let report = sweep(&e.pool, &e.policy, &only(&[lab, by_search_text]))
        .await
        .unwrap();
    assert_eq!(report.scanned, 2);
    assert_eq!(class_and_gen(&e, lab).await.0.as_deref(), Some("none"));
    assert_eq!(
        class_and_gen(&e, by_search_text).await.0.as_deref(),
        Some("text_marker")
    );

    for (column, value) in [
        ("name", "clientco"),
        ("git_remote_url", "gitea.example/clientco/repo"),
        ("git_main_path", "/w/clientco"),
    ] {
        let project: i64 = sqlx::query_scalar(&format!(
            "INSERT INTO projects (machine_id, provider, project_path, {column})
             VALUES ($1, 'claude', $2, $3) RETURNING id"
        ))
        .bind(e.machine)
        .bind(format!("/w/lab-{}", Uuid::new_v4()))
        .bind(value)
        .fetch_one(&e.pool)
        .await
        .unwrap();
        let s = session(&e, &["plain lab work"]).await;
        sqlx::query("UPDATE sessions SET project_id = $2 WHERE id = $1")
            .bind(s)
            .bind(project)
            .execute(&e.pool)
            .await
            .unwrap();
        sweep(&e.pool, &e.policy, &only(&[s])).await.unwrap();
        assert_eq!(
            class_and_gen(&e, s).await.0.as_deref(),
            Some("path"),
            "{column}"
        );
    }
}

/// E1 reaches history: a session already stamped lab at the current
/// generation is not read again by a plain sweep, and `rescan` reads it under
/// the current extractor, raise-only.
#[tokio::test]
async fn a_rescan_reads_sessions_already_stamped_lab() {
    let e = env().await;
    let s = session(&e, &["plain lab work"]).await;
    sweep(&e.pool, &e.policy, &only(&[s])).await.unwrap();
    assert_eq!(class_and_gen(&e, s).await.0.as_deref(), Some("none"));
    // A record the old extractor did not read, already in the archive.
    sqlx::query("UPDATE messages SET search_text = 'notes in /w/clientco/x' WHERE session_id = $1")
        .bind(s)
        .execute(&e.pool)
        .await
        .unwrap();
    let plain = sweep(&e.pool, &e.policy, &only(&[s])).await.unwrap();
    assert_eq!(plain.scanned, 0, "a plain sweep skips a stamped session");
    assert_eq!(class_and_gen(&e, s).await.0.as_deref(), Some("none"));

    let rescan = SweepOptions {
        rescan: true,
        ..only(&[s])
    };
    let report = sweep(&e.pool, &e.policy, &rescan).await.unwrap();
    assert_eq!(report.scanned, 1);
    assert_eq!(
        class_and_gen(&e, s).await,
        (
            Some("text_marker".into()),
            Some(e.policy.generation.clone())
        )
    );
}
