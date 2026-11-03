//! security fe3fd22 F1: a web page ac opens must not read the archive through
//! the Tailscale identity header that tailscale serve adds to his requests.
//! (a) CORS only for configured pages, never `*`; (b) the header counts only
//! for the hub's own Host and the hub's own Origin or none; (c) a presented
//! but dead cookie is a terminal 401. Refusals are counted by route template.
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use std::collections::HashMap;

use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;

const LOGIN: &str = "someone@example.test";
const OWN: &str = "https://hub.example:8788";
const ALLOWED_PAGE: &str = "http://localhost:3727";

/// A hub that trusts `LOGIN`, names itself `OWN` (plus its test address, so the
/// harness can reach it), and allows `ALLOWED_PAGE` cross-origin.
async fn spawn(public: bool) -> String {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origins = if public {
        vec![OWN.to_string(), format!("http://{addr}")]
    } else {
        Vec::new()
    };
    let state = hub::AppState::new(pool, HashMap::new(), vec![LOGIN.to_string()])
        .with_public_origins(origins)
        .with_cors_origins(vec![ALLOWED_PAGE.to_string()]);
    tokio::spawn(async move {
        axum::serve(listener, hub::router(state, None))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

/// GET /v1/projects?limit=1 with the identity header and the given Host,
/// Origin and Cookie: (status, Access-Control-Allow-Origin).
async fn read(
    base: &str,
    host: Option<&str>,
    origin: Option<&str>,
    cookie: Option<&str>,
) -> (u16, Option<String>) {
    let mut req = reqwest::Client::new()
        .get(format!("{base}/v1/projects?limit=1"))
        .header("Tailscale-User-Login", LOGIN);
    if let Some(h) = host {
        req = req.header("Host", h);
    }
    if let Some(o) = origin {
        req = req.header("Origin", o);
    }
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    let r = req.send().await.unwrap();
    let acao = r
        .headers()
        .get("access-control-allow-origin")
        .map(|v| v.to_str().unwrap().to_string());
    (r.status().as_u16(), acao)
}

#[tokio::test]
async fn the_header_reads_from_the_hubs_own_page_or_no_page() {
    let base = spawn(true).await;
    assert_eq!(
        read(&base, None, None, None).await.0,
        200,
        "no page, own address"
    );
    assert_eq!(
        read(&base, Some("hub.example:8788"), None, None).await.0,
        200
    );
    assert_eq!(
        read(&base, Some("hub.example:8788"), Some(OWN), None)
            .await
            .0,
        200
    );
}

#[tokio::test]
async fn a_foreign_page_gets_401_and_no_cors_grant() {
    let base = spawn(true).await;
    let (status, acao) = read(
        &base,
        Some("hub.example:8788"),
        Some("https://evil.example"),
        None,
    )
    .await;
    assert_eq!(status, 401);
    assert_eq!(acao, None, "never `*`, never a reflection");
    // The preflight a hostile page would need for any custom header: no grant.
    let pre = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("{base}/v1/projects"))
        .header("Origin", "https://evil.example")
        .header("Access-Control-Request-Method", "GET")
        .header("Access-Control-Request-Headers", "authorization")
        .send()
        .await
        .unwrap();
    assert!(pre.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn a_foreign_host_gets_401_dns_rebinding() {
    let base = spawn(true).await;
    assert_eq!(
        read(&base, Some("evil.example:8790"), None, None).await.0,
        401
    );
}

#[tokio::test]
async fn a_dead_cookie_does_not_fall_back_to_the_header() {
    let base = spawn(true).await;
    let (status, _) = read(
        &base,
        Some("hub.example:8788"),
        None,
        Some("cchv_session=revoked-or-made-up"),
    )
    .await;
    assert_eq!(status, 401);
}

#[tokio::test]
async fn with_no_public_origins_the_header_never_counts() {
    let base = spawn(false).await;
    assert_eq!(read(&base, None, None, None).await.0, 401);
}

#[tokio::test]
async fn only_configured_pages_get_cors_and_never_credentials() {
    let base = spawn(true).await;
    let r = reqwest::Client::new()
        .get(format!("{base}/v1/healthz"))
        .header("Origin", ALLOWED_PAGE)
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some(ALLOWED_PAGE)
    );
    assert!(r
        .headers()
        .get("access-control-allow-credentials")
        .is_none());
    let r = reqwest::Client::new()
        .get(format!("{base}/v1/healthz"))
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert!(r.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn refusals_are_counted_by_route_template_only() {
    let base = spawn(true).await;
    for _ in 0..2 {
        read(&base, Some("evil.example:8790"), None, None).await;
    }
    let r = reqwest::Client::new()
        .get(format!("{base}/v1/sessions/12345/messages"))
        .header("Tailscale-User-Login", LOGIN)
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    let body = reqwest::get(format!("{base}/v1/healthz/origin"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["total"].as_u64(), Some(3), "{body}");
    assert_eq!(v["by_route"]["/v1/projects"].as_u64(), Some(2), "{body}");
    assert_eq!(
        v["by_route"]["/v1/sessions/{id}/messages"].as_u64(),
        Some(1),
        "the route template, not the path: {body}"
    );
    assert!(
        !body.contains("12345") && !body.contains(LOGIN),
        "no path, no login: {body}"
    );
}
