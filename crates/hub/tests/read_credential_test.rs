//! openspec `hub-read-needs-credential`, release A: requests are counted by
//! the kind of credential that let them in (D1), and failed credential checks
//! on all routes share one limit that never refuses a valid credential (D4).
//!
//! Requires a reachable Postgres via `TEST_DATABASE_URL` (or `DATABASE_URL`).

use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use tokio::net::TcpListener;
use uuid::Uuid;

const LOGIN: &str = "someone@example.test";

struct Hub {
    base: String,
    token: String,
}

async fn spawn() -> Hub {
    let url = std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("TEST_DATABASE_URL or DATABASE_URL must be set for hub integration tests");
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("connect test db");
    hub::MIGRATOR.run(&pool).await.expect("run migrations");
    let machine = Uuid::new_v4();
    let token = format!("tok-{}-{}", machine.simple(), Uuid::new_v4().simple());
    let state = hub::AppState::new(
        pool,
        HashMap::from([(token.clone(), machine)]),
        vec![LOGIN.to_string()],
    );
    let app = hub::router(state, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Hub {
        base: format!("http://{addr}"),
        token,
    }
}

async fn get(hub: &Hub, path: &str, auth: Option<(&str, &str)>) -> reqwest::Response {
    let mut req = reqwest::Client::new().get(format!("{}{path}", hub.base));
    if let Some((name, value)) = auth {
        req = req.header(name, value);
    }
    req.send().await.unwrap()
}

/// D1: each credential kind is counted, a failure is counted, and the answer
/// names no login and no token.
#[tokio::test]
async fn healthz_auth_counts_by_kind_and_names_no_one() {
    let hub = spawn().await;
    let bearer = format!("Bearer {}", hub.token);
    assert_eq!(
        get(
            &hub,
            "/v1/projects?limit=1",
            Some(("authorization", &bearer))
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        get(
            &hub,
            "/v1/projects?limit=1",
            Some(("tailscale-user-login", LOGIN))
        )
        .await
        .status(),
        200
    );
    assert_eq!(get(&hub, "/v1/projects?limit=1", None).await.status(), 401);
    let r = get(&hub, "/v1/healthz/auth", None).await;
    assert_eq!(r.status(), 200);
    let body = r.text().await.unwrap();
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(v["accepted"]["bearer"].as_u64().unwrap() >= 1, "{body}");
    assert_eq!(v["accepted"]["tailscale"].as_u64(), Some(1), "{body}");
    assert_eq!(v["failed"].as_u64(), Some(1), "{body}");
    assert!(!body.contains(LOGIN), "no login in the answer: {body}");
    assert!(!body.contains(&hub.token), "no token in the answer: {body}");
}

/// D4: past the global limit a failing credential gets 429 with Retry-After,
/// on any route, while a valid credential is still served.
#[tokio::test]
async fn past_the_limit_a_failing_credential_gets_429_and_a_valid_one_is_served() {
    let hub = spawn().await;
    let wrong = format!("Bearer wrong-{}", Uuid::new_v4().simple());
    let mut last = 0;
    for i in 0..=hub::auth_stats::FAIL_LIMIT {
        // Alternate a read route and the login route: one shared window.
        let path = if i % 2 == 0 {
            "/v1/projects?limit=1"
        } else {
            "/v1/sessions?limit=1"
        };
        last = get(&hub, path, Some(("authorization", &wrong)))
            .await
            .status()
            .as_u16();
    }
    assert_eq!(last, 429, "the failure past the limit is refused");
    let r = get(
        &hub,
        "/v1/projects?limit=1",
        Some(("authorization", &wrong)),
    )
    .await;
    assert_eq!(r.status(), 429);
    assert!(
        r.headers().get("retry-after").is_some(),
        "Retry-After is set"
    );
    let bearer = format!("Bearer {}", hub.token);
    assert_eq!(
        get(
            &hub,
            "/v1/projects?limit=1",
            Some(("authorization", &bearer))
        )
        .await
        .status(),
        200,
        "a valid credential is never limited"
    );
    let v: Value = get(&hub, "/v1/healthz/auth", None)
        .await
        .json()
        .await
        .unwrap();
    assert!(v["limited"].as_u64().unwrap() >= 2, "{v}");
}
