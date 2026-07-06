//! Integration tests for Tailscale-identity read auth
//! (`openspec/specs/archive-search-api/spec.md`, "Authentication and
//! pagination"). Requires `TEST_DATABASE_URL`/`DATABASE_URL`.

use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;

const LOGIN: &str = "acolangelo1@gmail.com";

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

async fn spawn(trusted: Vec<String>) -> String {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&test_db_url())
        .await
        .expect("connect");
    hub::MIGRATOR.run(&pool).await.expect("migrate");

    let state = hub::AppState {
        pool,
        tokens: Arc::new(HashMap::new()),
        trusted_identities: Arc::new(trusted),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, hub::router(state, None))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn trusted_identity_header_grants_read() {
    let base = spawn(vec![LOGIN.to_string()]).await;
    let client = reqwest::Client::new();
    let res = client
        .get(format!("{base}/v1/projects"))
        .header("Tailscale-User-Login", LOGIN)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn untrusted_identity_is_401() {
    let base = spawn(vec![LOGIN.to_string()]).await;
    let client = reqwest::Client::new();
    let res = client
        .get(format!("{base}/v1/projects"))
        .header("Tailscale-User-Login", "mallory@example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn empty_allowlist_ignores_identity_header() {
    let base = spawn(Vec::new()).await;
    let client = reqwest::Client::new();
    let res = client
        .get(format!("{base}/v1/projects"))
        .header("Tailscale-User-Login", LOGIN)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn ingest_stays_bearer_only() {
    let base = spawn(vec![LOGIN.to_string()]).await;
    let client = reqwest::Client::new();
    let res = client
        .post(format!("{base}/v1/ingest"))
        .header("Tailscale-User-Login", LOGIN)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}
