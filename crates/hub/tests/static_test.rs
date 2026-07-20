//! Integration tests for optional static-dir hosting
//! (`openspec/specs/hub-static-hosting/spec.md`).
//!
//! The static fallback is orthogonal to archive data, so these tests only
//! need a database pool for `AppState`/`/v1/healthz` — no fixtures. Requires
//! `TEST_DATABASE_URL`/`DATABASE_URL` like the other integration tests.

use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::net::TcpListener;
use uuid::Uuid;

fn test_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
}

/// Build a throwaway static root with an index, an asset, and a decoy `v1/`
/// tree that must never shadow the API.
fn make_static_root() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hub-static-{}", Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::create_dir_all(dir.join("v1")).unwrap();
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><title>archive</title>",
    )
    .unwrap();
    std::fs::write(dir.join("assets/app.css"), "body{color:red}").unwrap();
    std::fs::write(dir.join("v1/healthz"), "STATIC DECOY — must not be served").unwrap();
    dir
}

async fn spawn(static_dir: Option<PathBuf>) -> String {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&test_db_url())
        .await
        .expect("connect");
    hub::MIGRATOR.run(&pool).await.expect("migrate");

    let state = hub::AppState::new(pool, HashMap::new(), Vec::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, hub::router(state, static_dir.as_deref()))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn serves_index_and_assets_when_configured() {
    let root = make_static_root();
    let base = spawn(Some(root)).await;
    let client = reqwest::Client::new();

    // `/` resolves to index.html, no Authorization header required, and must
    // always revalidate so a webapp update is picked up without a hard reload.
    let res = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(res.status(), 200);
    let ct = res.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ct.starts_with("text/html"), "content-type was {ct}");
    assert_eq!(res.headers()["cache-control"], "no-cache");
    assert!(res.text().await.unwrap().contains("archive"));

    // Content-hashed assets are served immutable so browsers skip revalidation.
    let res = client
        .get(format!("{base}/assets/app.css"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let ct = res.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ct.starts_with("text/css"), "content-type was {ct}");
    assert_eq!(
        res.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn api_routes_win_over_static_decoys() {
    let root = make_static_root();
    let base = spawn(Some(root)).await;

    // The static root contains `v1/healthz`; the JSON handler must still win.
    let res = reqwest::get(format!("{base}/v1/healthz")).await.unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn unconfigured_hub_keeps_plain_404_root() {
    let base = spawn(None).await;
    let res = reqwest::get(format!("{base}/")).await.unwrap();
    assert_eq!(res.status(), 404);
}
