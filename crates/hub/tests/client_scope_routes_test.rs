//! The route walk (openspec `hub-client-read-scope`, D6, R2-J): every GET route
//! the router registers has a case here, and no lab-read principal ever gets a
//! client canary back. A positive control proves each content case reached its
//! handler: the lab canary comes back where it should, and a client reader
//! sees the client canary.
//!
//! Requires `TEST_DATABASE_URL`; run with `--test-threads=1`.

use std::collections::BTreeSet;

use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use uuid::Uuid;

const ROUTING: &str = r#"
[protected]
model = "claude-x"
path_markers = ["/clientco"]
content_terms = []
"#;
const LOGIN: &str = "ac@example.test";

fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    Sha256::digest(b).iter().fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

/// Every GET path in `src/lib.rs`'s router: the registry this test must cover.
fn registered_get_paths() -> BTreeSet<String> {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs")).unwrap();
    let mut out = BTreeSet::new();
    let mut rest = src.as_str();
    while let Some(i) = rest.find(".route(") {
        rest = &rest[i + 7..];
        let body: String = rest.chars().take_while(|c| *c != ';').collect();
        let path = body.split('"').nth(1).unwrap().to_string();
        let end = body.find("\n        .route(").unwrap_or(body.len());
        if body[..end].contains("get(") {
            out.insert(path);
        }
    }
    out
}

struct World {
    base: String,
    lab_reader: String,
    client_reader: String,
    lab_canary: String,
    client_canary: String,
    client_path: String,
    lab_path: String,
    client_id: i64,
    lab_id: i64,
    client_sid: String,
    _dir: std::path::PathBuf,
}

async fn world() -> World {
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
    let dir = std::env::temp_dir().join(format!("cchv-cs-routes-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("r.toml"), ROUTING).unwrap();
    std::fs::write(
        dir.join("f.json"),
        json!({"sha256": hex(ROUTING.as_bytes()), "plain_path_markers": 1,
               "encoded_path_markers": 0, "content_terms": 0})
        .to_string(),
    )
    .unwrap();

    let r = Uuid::new_v4().simple().to_string();
    let (machine, machine_token) = (Uuid::new_v4(), format!("tok-m-{r}"));
    let lab_reader = format!("tok-lab-{r}");
    let client_reader = format!("tok-client-{r}");
    let config: hub::HubConfig = toml::from_str(&format!(
        "database_url = \"unused\"\n\
         trust_tailscale_identity = [\"{LOGIN}\"]\n\
         [[tokens]]\ntoken = \"{machine_token}\"\nmachine_id = \"{machine}\"\nlabel = \"m\"\n\
         [[readers]]\ntoken = \"{lab_reader}\"\nmachine_id = \"{}\"\nlabel = \"blog\"\n\
         [[client_readers]]\ntoken_sha256 = \"{}\"\nmachine_id = \"{}\"\nlabel = \"distiller\"\n\
         [client_scope]\nrouting_file = \"{}\"\nfloor_file = \"{}\"\nenforce = true\n",
        Uuid::new_v4(),
        hex(client_reader.as_bytes()),
        Uuid::new_v4(),
        dir.join("r.toml").display(),
        dir.join("f.json").display(),
    ))
    .unwrap();
    config.validate().unwrap();
    let state = hub::AppState::from_config(pool.clone(), &config).unwrap();
    state
        .client_scope
        .as_ref()
        .unwrap()
        .refresh(&pool)
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = hub::router(state, None);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let lab_canary = format!("labcanary{r}");
    let client_canary = format!("clientcanary{r}");
    let lab_path = format!("/w/labproj-{r}");
    let client_path = format!("/w/clientco/proj-{r}");
    let (lab_sid, client_sid) = (Uuid::new_v4().to_string(), Uuid::new_v4().to_string());
    for (path, sid, canary) in [
        (&lab_path, &lab_sid, &lab_canary),
        (&client_path, &client_sid, &client_canary),
    ] {
        let resp = reqwest::Client::new()
            .post(format!("{base}/v1/ingest"))
            .bearer_auth(&machine_token)
            .json(&json!({
                "machine": {"machine_id": machine, "hostname": format!("rt-{}", &r[..8]), "os": "macos"},
                "projects": [{"provider": "claude", "project_path": path, "name": canary}],
                "sessions": [{"provider": "claude", "session_id": sid, "project_path": path, "summary": canary}],
                "messages": [{"provider": "claude", "session_id": sid, "message_key": Uuid::new_v4().to_string(),
                              "seq": 0, "timestamp": "2026-10-30T12:00:00Z", "type": "user", "role": "user",
                              "content": [{"type": "text", "text": format!("about {canary} today")}],
                              "raw": {"cwd": path}, "search_text": format!("about {canary} today")}],
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
    }
    // A pending day (closed, inside the 7-day horizon, no entry) for both.
    let two_days_ago = (chrono::Utc::now() - chrono::Duration::days(2))
        .format("%Y-%m-%dT12:00:00Z")
        .to_string();
    for (path, sid, canary) in [
        (&lab_path, &lab_sid, &lab_canary),
        (&client_path, &client_sid, &client_canary),
    ] {
        let resp = reqwest::Client::new()
            .post(format!("{base}/v1/ingest"))
            .bearer_auth(&machine_token)
            .json(&json!({
                "machine": {"machine_id": machine, "hostname": format!("rt-{}", &r[..8]), "os": "macos"},
                "projects": [{"provider": "claude", "project_path": path, "name": "p"}],
                "sessions": [{"provider": "claude", "session_id": sid, "project_path": path, "summary": canary}],
                "messages": [{"provider": "claude", "session_id": sid, "message_key": Uuid::new_v4().to_string(),
                              "seq": 1, "timestamp": two_days_ago, "type": "user", "role": "user",
                              "content": [{"type": "text", "text": "later"}], "raw": {"cwd": path}}],
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
    }
    let id = |sid: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT id FROM sessions WHERE session_id = $1")
                .bind(sid)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let (lab_id, client_id) = (id(lab_sid.clone()).await, id(client_sid.clone()).await);
    for (path, sid, canary, route) in [
        (&lab_path, lab_id, &lab_canary, "general"),
        (&client_path, client_id, &client_canary, "protected"),
    ] {
        let resp = reqwest::Client::new()
            .post(format!("{base}/v1/journal/entries"))
            .bearer_auth(&machine_token)
            .json(&json!({
                "entry_date": "2026-10-30", "project_path": path, "status": "entry",
                "headline": format!("entry {canary}"), "summary": format!("summary {canary}"),
                "topics": ["one", "two", "three"], "open_questions": [format!("question {canary}")],
                "session_ids": [sid], "model": "m", "route": route,
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200, "journal entry create");
    }
    World {
        base,
        lab_reader,
        client_reader,
        lab_canary,
        client_canary,
        client_path,
        lab_path,
        client_id,
        lab_id,
        client_sid,
        _dir: dir,
    }
}

/// The concrete requests for every registered GET path. `true` marks a
/// content case: it must return the lab canary to a lab reader.
fn cases(w: &World) -> Vec<(&'static str, String, Option<String>)> {
    let c = &w.client_canary;
    let l = &w.lab_canary;
    let cp = urlencode(&w.client_path);
    let lab = || Some(l.clone());
    let lab_path = || Some(w.lab_path.clone());
    vec![
        ("/v1/healthz", "/v1/healthz".into(), None),
        ("/v1/healthz/ingest", "/v1/healthz/ingest".into(), None),
        (
            "/v1/healthz/journal",
            "/v1/healthz/journal?within_days=7".into(),
            lab_path(),
        ),
        ("/v1/healthz/stats", "/v1/healthz/stats".into(), None),
        ("/v1/healthz/findings", "/v1/healthz/findings".into(), None),
        ("/v1/findings/summary", "/v1/findings/summary".into(), None),
        ("/v1/search", format!("/v1/search?q={c}"), None),
        (
            "/v1/search",
            format!("/v1/search?q={c}&scope=journal"),
            None,
        ),
        ("/v1/search", format!("/v1/search?q={l}"), lab()),
        (
            "/v1/journal/pending",
            "/v1/journal/pending?from=2026-10-01&limit=500".into(),
            lab_path(),
        ),
        (
            "/v1/journal/entries",
            "/v1/journal/entries?from=2026-10-01".into(),
            lab(),
        ),
        (
            "/v1/journal/entries",
            format!("/v1/journal/entries?project={cp}"),
            None,
        ),
        (
            "/v1/journal/open-questions",
            "/v1/journal/open-questions?days=3650".into(),
            None,
        ),
        (
            "/v1/journal/open-questions",
            format!("/v1/journal/open-questions?days=3650&project={cp}"),
            None,
        ),
        ("/v1/projects", "/v1/projects?limit=500".into(), None),
        ("/v1/sessions", "/v1/sessions?limit=500".into(), None),
        ("/v1/sessions", format!("/v1/sessions?project={cp}"), None),
        (
            "/v1/sessions/{id}/messages",
            format!("/v1/sessions/{}/messages", w.client_id),
            None,
        ),
        (
            "/v1/sessions/{id}/messages",
            format!("/v1/sessions/{}/messages", w.client_sid),
            None,
        ),
        (
            "/v1/sessions/{id}/messages",
            format!("/v1/sessions/{}/messages", w.lab_id),
            lab(),
        ),
        ("/v1/stats/global", "/v1/stats/global".into(), None),
        (
            "/v1/stats/projects/{identity_key}",
            format!("/v1/stats/projects/{cp}"),
            None,
        ),
        (
            "/v1/stats/sessions/{id}",
            format!("/v1/stats/sessions/{}", w.client_id),
            None,
        ),
        ("/v1/identities", "/v1/identities".into(), None),
    ]
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn get(w: &World, url: &str, who: &str) -> (u16, String) {
    let mut req = reqwest::Client::new().get(format!("{}{url}", w.base));
    req = match who {
        "lab" => req.bearer_auth(&w.lab_reader),
        "client" => req.bearer_auth(&w.client_reader),
        "tailscale" => req.header("Tailscale-User-Login", LOGIN),
        _ => req,
    };
    let r = req.send().await.unwrap();
    (r.status().as_u16(), r.text().await.unwrap())
}

#[tokio::test]
async fn every_registered_get_route_has_a_case() {
    let w = world().await;
    let covered: BTreeSet<String> = cases(&w).iter().map(|(p, _, _)| (*p).to_string()).collect();
    let missing: Vec<String> = registered_get_paths()
        .difference(&covered)
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "GET routes with no client-scope case: {missing:?}"
    );
}

#[tokio::test]
async fn no_lab_principal_gets_a_client_canary() {
    let w = world().await;
    let mut leaks = Vec::new();
    let mut unreached = Vec::new();
    for (_, url, content) in cases(&w) {
        for who in ["lab", "tailscale", "none"] {
            let (status, body) = get(&w, &url, who).await;
            // A path the request itself carries may be echoed back (the same
            // answer as for a project that does not exist); anything else is a leak.
            let echoed = url.contains(&urlencode(&w.client_path));
            if body.contains(&w.client_canary) || (!echoed && body.contains(&w.client_path)) {
                leaks.push(format!("{who} {url} -> {status}"));
            }
            if let Some(want) = content.as_ref().filter(|_| who != "none") {
                if !body.contains(want.as_str()) {
                    unreached.push(format!("{who} {url} -> {status}"));
                }
            }
        }
    }
    assert!(
        leaks.is_empty(),
        "client material served to lab readers: {leaks:?}"
    );
    assert!(
        unreached.is_empty(),
        "content cases that did not reach their handler: {unreached:?}"
    );
}

#[tokio::test]
async fn a_client_reader_sees_client_sessions() {
    let w = world().await;
    let mut seen = 0;
    for url in [
        format!("/v1/sessions/{}/messages", w.client_id),
        format!("/v1/search?q={}", w.client_canary),
        "/v1/sessions?limit=500".to_string(),
        format!("/v1/journal/entries?project={}", urlencode(&w.client_path)),
    ] {
        let (status, body) = get(&w, &url, "client").await;
        assert_eq!(status, 200, "{url}");
        if body.contains(&w.client_canary) {
            seen += 1;
        }
    }
    assert_eq!(seen, 4, "the client reader reads every client case");
    // And a client reader is read-only.
    let r = reqwest::Client::new()
        .post(format!("{}/v1/journal/ticks", w.base))
        .bearer_auth(&w.client_reader)
        .json(&json!({"mode": "forward"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
}
