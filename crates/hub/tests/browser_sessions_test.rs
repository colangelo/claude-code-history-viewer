//! openspec `hub-read-needs-credential` D3 with security eff460e1: browser
//! sessions from `/v1/login` survive a hub restart, take their scope from the
//! current config, rotate, are capped, revocable and expire; only reader
//! tokens log in.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;

struct Ids {
    reader: String,
    reader_id: Uuid,
    stats: String,
    machine: String,
}

fn ids() -> Ids {
    let t = || format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    Ids {
        reader: t(),
        reader_id: Uuid::new_v4(),
        stats: t(),
        machine: t(),
    }
}

fn config(url: &str, ids: &Ids, with_reader: bool) -> hub::config::HubConfig {
    let reader = if with_reader {
        format!(
            "[[readers]]\ntoken = \"{}\"\nmachine_id = \"{}\"\nlabel = \"browser\"\n",
            ids.reader, ids.reader_id
        )
    } else {
        String::new()
    };
    toml::from_str(&format!(
        "database_url = \"{url}\"\n\
         [[tokens]]\ntoken = \"{}\"\nmachine_id = \"{}\"\n\
         {reader}\
         [[readers]]\ntoken = \"{}\"\nmachine_id = \"{}\"\nlabel = \"stats\"\nstats_only = true\n",
        ids.machine,
        Uuid::new_v4(),
        ids.stats,
        Uuid::new_v4(),
    ))
    .unwrap()
}

async fn pool() -> (PgPool, String) {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests");
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");
    (pool, url)
}

/// A hub on `cfg` with fingerprint key `key`: a "restart" is another call.
async fn spawn(pool: &PgPool, cfg: &hub::config::HubConfig, key: [u8; 32]) -> String {
    let mut state = hub::AppState::from_config(pool.clone(), cfg).unwrap();
    state.logins = state.logins.with_key(key);
    let app = hub::router(state, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// POST /v1/login: (status, the `name=value` cookie when one is set).
async fn login(base: &str, token: &str, cookie: Option<&str>) -> (u16, Option<String>) {
    let mut req = reqwest::Client::new()
        .post(format!("{base}/v1/login"))
        .bearer_auth(token);
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    let r = req.send().await.unwrap();
    let set = r
        .headers()
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
    (r.status().as_u16(), set)
}

async fn read(base: &str, cookie: &str) -> u16 {
    reqwest::Client::new()
        .get(format!("{base}/v1/projects?limit=1"))
        .header("Cookie", cookie)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

fn id_hash(cookie: &str) -> Vec<u8> {
    let value = cookie.split_once('=').unwrap().1;
    Sha256::digest(value.as_bytes()).to_vec()
}

#[tokio::test]
async fn only_a_lab_reader_logs_in_and_gets_a_host_cookie() {
    let (pool, url) = pool().await;
    let ids = ids();
    let base = spawn(&pool, &config(&url, &ids, true), [7; 32]).await;
    assert_eq!(
        login(&base, &ids.machine, None).await.0,
        401,
        "a machine token never logs in"
    );
    assert_eq!(
        login(&base, &ids.stats, None).await.0,
        403,
        "a stats-only reader never logs in"
    );
    let r = reqwest::Client::new()
        .post(format!("{base}/v1/login"))
        .bearer_auth(&ids.reader)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 204);
    let set = r.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(set.starts_with("__Host-cchv_session="), "{set}");
    assert!(set.contains("Path=/") && set.contains("Secure") && set.contains("HttpOnly"));
    assert!(!set.to_ascii_lowercase().contains("domain="), "{set}");
    let cookie = set.split(';').next().unwrap();
    assert_eq!(read(&base, cookie).await, 200);
    // The table holds the hash of the value, never the value.
    let value = cookie.split_once('=').unwrap().1;
    let count = |q: &'static str, v: Vec<u8>| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(q)
                .bind(v)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let by_hash = "SELECT count(*) FROM browser_sessions WHERE id_hash = $1";
    assert_eq!(count(by_hash, id_hash(cookie)).await, 1);
    assert_eq!(count(by_hash, value.as_bytes().to_vec()).await, 0);
}

#[tokio::test]
async fn a_session_survives_a_restart_under_the_same_key_only() {
    let (pool, url) = pool().await;
    let ids = ids();
    let cfg = config(&url, &ids, true);
    let first = spawn(&pool, &cfg, [1; 32]).await;
    let (_, cookie) = login(&first, &ids.reader, None).await;
    let cookie = cookie.unwrap();
    let restarted = spawn(&pool, &cfg, [1; 32]).await;
    assert_eq!(
        read(&restarted, &cookie).await,
        200,
        "a restart logs nobody out"
    );
    let other_key = spawn(&pool, &cfg, [2; 32]).await;
    assert_eq!(
        read(&other_key, &cookie).await,
        401,
        "the fingerprint is keyed"
    );
}

#[tokio::test]
async fn removing_the_token_from_the_config_ends_its_sessions() {
    let (pool, url) = pool().await;
    let ids = ids();
    let before = spawn(&pool, &config(&url, &ids, true), [3; 32]).await;
    let (_, cookie) = login(&before, &ids.reader, None).await;
    let cookie = cookie.unwrap();
    assert_eq!(read(&before, &cookie).await, 200);
    let after = spawn(&pool, &config(&url, &ids, false), [3; 32]).await;
    assert_eq!(read(&after, &cookie).await, 401);
}

#[tokio::test]
async fn a_login_with_a_cookie_replaces_it() {
    let (pool, url) = pool().await;
    let ids = ids();
    let base = spawn(&pool, &config(&url, &ids, true), [4; 32]).await;
    let (_, old) = login(&base, &ids.reader, None).await;
    let old = old.unwrap();
    let (status, new) = login(&base, &ids.reader, Some(&old)).await;
    assert_eq!(status, 204);
    assert_eq!(read(&base, &old).await, 401, "the replaced cookie is over");
    assert_eq!(read(&base, &new.unwrap()).await, 200);
}

#[tokio::test]
async fn a_principal_keeps_at_most_five_sessions() {
    let (pool, url) = pool().await;
    let ids = ids();
    let base = spawn(&pool, &config(&url, &ids, true), [5; 32]).await;
    let mut cookies = Vec::new();
    for _ in 0..=hub::login::MAX_PER_PRINCIPAL {
        cookies.push(login(&base, &ids.reader, None).await.1.unwrap());
    }
    assert_eq!(read(&base, &cookies[0]).await, 401, "the oldest is dropped");
    for c in &cookies[1..] {
        assert_eq!(read(&base, c).await, 200);
    }
}

#[tokio::test]
async fn delete_and_the_cli_revoke_sessions() {
    let (pool, url) = pool().await;
    let ids = ids();
    let base = spawn(&pool, &config(&url, &ids, true), [6; 32]).await;
    let (_, a) = login(&base, &ids.reader, None).await;
    let a = a.unwrap();
    let r = reqwest::Client::new()
        .delete(format!("{base}/v1/login"))
        .header("Cookie", &a)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 204);
    assert_eq!(read(&base, &a).await, 401, "signed out");

    let (_, b) = login(&base, &ids.reader, None).await;
    let b = b.unwrap();
    assert_eq!(read(&base, &b).await, 200);
    let principal = format!("machine:{}", ids.reader_id);
    assert_eq!(
        hub::login::revoke(&pool, Some(&principal)).await.unwrap(),
        1
    );
    assert_eq!(read(&base, &b).await, 401, "revoked by the CLI");
}

#[tokio::test]
async fn a_session_ends_after_seven_idle_days_or_thirty_in_all() {
    let (pool, url) = pool().await;
    let ids = ids();
    let base = spawn(&pool, &config(&url, &ids, true), [8; 32]).await;
    let (_, idle) = login(&base, &ids.reader, None).await;
    let idle = idle.unwrap();
    let (_, old) = login(&base, &ids.reader, None).await;
    let old = old.unwrap();
    assert_eq!(read(&base, &idle).await, 200);
    assert_eq!(read(&base, &old).await, 200);
    sqlx::query("UPDATE browser_sessions SET last_seen = now() - interval '7 days 1 minute' WHERE id_hash = $1")
        .bind(id_hash(&idle))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE browser_sessions SET expires_at = now() - interval '1 second' WHERE id_hash = $1",
    )
    .bind(id_hash(&old))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(read(&base, &idle).await, 401, "seven idle days");
    assert_eq!(read(&base, &old).await, 401, "thirty days in all");
}
