//! The route walk (openspec `hub-client-read-scope`, D6, R2-J): every GET route
//! the router registers has a case here, and no lab-read principal ever gets a
//! client canary back. A positive control proves each content case reached its
//! handler: the lab canary comes back where it should, and a client reader
//! sees the client canary.
//!
//! Requires `TEST_DATABASE_URL`; run with `--test-threads=1`.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{json, Value};
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
    /// A `[[readers]]` token with `stats_only = true` (stats-read-api D3 c3).
    stats_reader: String,
    client_reader: String,
    lab_canary: String,
    client_canary: String,
    client_path: String,
    lab_path: String,
    client_id: i64,
    lab_id: i64,
    client_sid: String,
    machine: Uuid,
    machine_token: String,
    /// A second machine that ingests only client sessions (security T1).
    client_machine: Uuid,
    client_machine_token: String,
    state: hub::AppState,
    /// The statistics mirror this hub serves from, refreshed once the world
    /// is ingested (openspec stats-read-api: the stats routes join the walk).
    mirror: Arc<hub::mirror::Mirror>,
    mirror_cfg: hub::config::MirrorConfig,
    /// `/v1/stats/daily` over the last 31 days, which hold the world's
    /// two-days-ago rows.
    daily_url: String,
    /// `/v1/stats/tool-calls` for the one tool both sessions called, two days ago.
    tool_calls_url: String,
    /// `/v1/journal/pending` from the world's own pending day (two days ago):
    /// pending is newest first, and the world sweeps the shared test database,
    /// so an early `from` lets other tests' groups push the world's off the page.
    pending_url: String,
    dir: std::path::PathBuf,
}

async fn world() -> World {
    world_with(true).await
}

/// The world under enforce (`true`) or observe (`false`).
async fn world_with(enforce: bool) -> World {
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
    let (client_machine, client_machine_token) = (Uuid::new_v4(), format!("tok-cm-{r}"));
    let lab_reader = format!("tok-lab-{r}");
    let stats_reader = format!("tok-stats-{r}");
    let client_reader = format!("tok-client-{r}");
    let config: hub::HubConfig = toml::from_str(&format!(
        "database_url = \"unused\"\n\
         trust_tailscale_identity = [\"{LOGIN}\"]\n\
         [[tokens]]\ntoken = \"{machine_token}\"\nmachine_id = \"{machine}\"\nlabel = \"m\"\n\
         [[tokens]]\ntoken = \"{client_machine_token}\"\nmachine_id = \"{client_machine}\"\nlabel = \"cm\"\n\
         [[readers]]\ntoken = \"{lab_reader}\"\nmachine_id = \"{}\"\nlabel = \"blog\"\n\
         [[readers]]\ntoken = \"{stats_reader}\"\nmachine_id = \"{}\"\nlabel = \"monitoring\"\nstats_only = true\n\
         [[client_readers]]\ntoken_sha256 = \"{}\"\nmachine_id = \"{}\"\nlabel = \"distiller\"\n\
         [client_scope]\nrouting_file = \"{}\"\nfloor_file = \"{}\"\nenforce = {enforce}\n",
        Uuid::new_v4(),
        Uuid::new_v4(),
        hex(client_reader.as_bytes()),
        Uuid::new_v4(),
        dir.join("r.toml").display(),
        dir.join("f.json").display(),
    ))
    .unwrap();
    config.validate().unwrap();
    let mirror_cfg = hub::config::MirrorConfig {
        path: Some(dir.join("stats.duckdb")),
        ..hub::config::MirrorConfig::default()
    };
    let mirror = Arc::new(hub::mirror::Mirror::open_or_create(&mirror_cfg).unwrap());
    let state = hub::AppState::from_config(pool.clone(), &config)
        .unwrap()
        .with_mirror(mirror.clone());
    state
        .client_scope
        .as_ref()
        .unwrap()
        .refresh(&pool)
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    // The hub's own name is the address the tests call (security fe3fd22 F1).
    let state = state.with_public_origins(vec![base.clone()]);
    let router = hub::router(state.clone(), None);
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
                "projects": [{"provider": "claude", "project_path": path, "name": canary,
                              "git_root_commit": hex(path.as_bytes())[..40].to_string(),
                              "git_remote_url": format!("https://example.test/{}.git", &hex(path.as_bytes())[..12])}],
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
    // One tool call in each session's newest (two-days-ago) message.
    let tool = format!("mcp__exa__rt_{}", &r[..8]);
    for sid in [lab_id, client_id] {
        sqlx::query(
            "INSERT INTO message_tool_uses (message_ref, session_id, seq, tool_name)
             SELECT id, session_id, 0, $2 FROM messages
              WHERE session_id = $1 ORDER BY \"timestamp\" DESC LIMIT 1",
        )
        .bind(sid)
        .bind(&tool)
        .execute(&pool)
        .await
        .unwrap();
    }
    let since = (chrono::Utc::now() - chrono::Duration::days(30))
        .format("%Y-%m-%dT%H%%3A%M%%3A%SZ")
        .to_string();
    let tool_calls_url = format!("/v1/stats/tool-calls?prefix=rt_{}&since={since}", &r[..8]);
    // The world is a swept hub: every session in the shared test database is
    // stamped under this generation, as production is after a sweep. Without
    // it, other tests' unscanned sessions make the stats routes say stale
    // (openspec stats-stale-after-generation).
    let hub::client_policy::Load::Ready(policy) =
        hub::client_policy::load_paths(&dir.join("r.toml"), &dir.join("f.json"))
    else {
        panic!("policy")
    };
    hub::client_sweep::sweep(
        &pool,
        &policy,
        &hub::client_sweep::SweepOptions {
            dry_run: false,
            batch: 500,
            only_sessions: None,
            rescan: false,
        },
    )
    .await
    .unwrap();
    mirror.refresh(&pool).await.unwrap();
    let today = chrono::Utc::now().date_naive();
    let daily_url = format!(
        "/v1/stats/daily?from={}&to={today}&tz=UTC",
        today - chrono::Duration::days(30)
    );
    let pending_url = format!(
        "/v1/journal/pending?from={}&limit=500",
        (chrono::Utc::now() - chrono::Duration::days(2)).format("%Y-%m-%d")
    );
    World {
        base,
        lab_reader,
        stats_reader,
        client_reader,
        lab_canary,
        client_canary,
        client_path,
        lab_path,
        client_id,
        lab_id,
        client_sid,
        machine,
        machine_token,
        client_machine,
        client_machine_token,
        state,
        mirror,
        mirror_cfg,
        daily_url,
        tool_calls_url,
        pending_url,
        dir,
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
        // Refused identity-header requests by route template (security fe3fd22 F1): no rows.
        ("/v1/healthz/origin", "/v1/healthz/origin".into(), None),
        // Counts by credential kind only (hub-read-needs-credential D1): no rows.
        ("/v1/healthz/auth", "/v1/healthz/auth".into(), None),
        (
            "/v1/healthz/client-scope",
            "/v1/healthz/client-scope".into(),
            Some("\"mode\":\"enforce\"".into()),
        ),
        ("/v1/findings/summary", "/v1/findings/summary".into(), None),
        ("/v1/search", format!("/v1/search?q={c}"), None),
        (
            "/v1/search",
            format!("/v1/search?q={c}&scope=journal"),
            None,
        ),
        ("/v1/search", format!("/v1/search?q={l}"), lab()),
        ("/v1/journal/pending", w.pending_url.clone(), lab_path()),
        (
            "/v1/journal/entries",
            "/v1/journal/entries?from=2026-10-01".into(),
            None,
        ),
        // The positive control asks for the lab project only: the shared test
        // database keeps gaining entries of the same date, so an unfiltered page
        // can end before the lab entry.
        (
            "/v1/journal/entries",
            format!("/v1/journal/entries?project={}", urlencode(&w.lab_path)),
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
        ("/v1/identities", "/v1/identities".into(), lab_path()),
        (
            "/v1/stats/daily",
            w.daily_url.clone(),
            Some("\"space\":\"ac\"".into()),
        ),
        (
            "/v1/stats/tool-calls",
            w.tool_calls_url.clone(),
            Some("\"count\":1".into()),
        ),
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
        "stats" => req.bearer_auth(&w.stats_reader),
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

/// stats-read-api D3 condition 3: a `stats_only` reader reads the two
/// stats-read routes and nothing else: 403 on every other GET case of the walk,
/// and no `/v1/login` cookie that would carry its read scope elsewhere.
#[tokio::test]
async fn a_stats_only_reader_reads_the_stats_routes_and_nothing_else() {
    let w = world().await;
    let allowed = ["/v1/stats/daily", "/v1/stats/tool-calls"];
    let mut wrong = Vec::new();
    for (path, url, _) in cases(&w) {
        let (status, body) = get(&w, &url, "stats").await;
        let want = if allowed.contains(&path) { 200 } else { 403 };
        if status != want {
            wrong.push(format!(
                "{url} -> {status} (want {want}) {}",
                &body[..body.len().min(120)]
            ));
        }
    }
    assert!(wrong.is_empty(), "stats_only reader: {wrong:#?}");
    for allowed_path in allowed {
        assert!(
            cases(&w).iter().any(|(p, _, _)| *p == allowed_path),
            "the walk has a case for {allowed_path}"
        );
    }
    let login = reqwest::Client::new()
        .post(format!("{}/v1/login", w.base))
        .bearer_auth(&w.stats_reader)
        .send()
        .await
        .unwrap();
    assert_eq!(
        login.status().as_u16(),
        403,
        "no cookie for a stats_only token"
    );
    let write = reqwest::Client::new()
        .post(format!("{}/v1/journal/ticks", w.base))
        .bearer_auth(&w.stats_reader)
        .json(&json!({"mode": "forward"}))
        .send()
        .await
        .unwrap();
    assert!(
        matches!(write.status().as_u16(), 401 | 403),
        "a stats_only token writes nothing: {}",
        write.status()
    );
    // The control: the ordinary lab reader is not refused on the same walk.
    let (status, _) = get(&w, "/v1/projects?limit=500", "lab").await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn a_client_reader_sees_client_sessions() {
    let w = world().await;
    let mut seen = 0;
    for url in [
        format!("/v1/sessions/{}/messages", w.client_id),
        format!("/v1/search?q={}", w.client_canary),
        // The world's own project: an unfiltered first page (200 at most) of
        // the shared test database need not hold it.
        format!(
            "/v1/sessions?project={}&limit=200",
            urlencode(&w.client_path)
        ),
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

/// R2-G: a lab caller naming a client project in an alias write gets the
/// same 404 as for a path with no project at all, before any other check.
#[tokio::test]
async fn alias_writes_do_not_reveal_client_projects() {
    let w = world().await;
    let key: String = {
        let body = get(&w, "/v1/identities", "client").await.1;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        v.as_array()
            .unwrap()
            .iter()
            .find(|id| {
                id["members"]
                    .as_array()
                    .is_some_and(|ms| ms.iter().any(|m| m["project_path"] == w.lab_path.as_str()))
            })
            .expect("the lab project's identity")["identity_key"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let post = |path: String| {
        let base = w.base.clone();
        let key = key.clone();
        async move {
            reqwest::Client::new()
                .post(format!("{base}/v1/identities/aliases"))
                .header("Tailscale-User-Login", LOGIN)
                .json(&json!({"project_path": path, "identity_key": key}))
                .send()
                .await
                .unwrap()
        }
    };
    let missing_path = format!("/w/no-such-project-{}", Uuid::new_v4());
    let client = post(w.client_path.clone()).await;
    let missing = post(missing_path.clone()).await;
    assert_eq!(client.status().as_u16(), 404);
    assert_eq!(missing.status().as_u16(), 404);
    let (a, b) = (client.text().await.unwrap(), missing.text().await.unwrap());
    assert_eq!(
        a.replace(&w.client_path, "<p>"),
        b.replace(&missing_path, "<p>"),
        "a client path and a missing one answer alike"
    );
    let lab = post(w.lab_path.clone()).await;
    assert_eq!(lab.status().as_u16(), 201, "a lab path still aliases");
}

/// R2-J: each lab response equals the same request after the client rows are
/// physically deleted: status, `X-Total-Count` and body. This catches leaks of
/// ids, counts, ranks and order, not only of strings.
#[tokio::test]
async fn lab_responses_equal_responses_with_client_rows_deleted() {
    let w = world().await;
    let cp = urlencode(&w.client_path);
    let (c, l) = (&w.client_canary, &w.lab_canary);
    let urls = vec![
        "/v1/projects?limit=500".to_string(),
        "/v1/sessions?limit=500".to_string(),
        format!("/v1/sessions?project={cp}"),
        format!("/v1/search?q={c}"),
        format!("/v1/search?q={l}"),
        format!("/v1/search?q={c}&scope=journal"),
        format!("/v1/search?q={c}&scope=messages&limit=1&offset=0"),
        "/v1/journal/entries?from=2026-10-01".to_string(),
        format!("/v1/journal/entries?project={cp}"),
        "/v1/journal/open-questions?days=3650".to_string(),
        format!("/v1/journal/open-questions?days=3650&project={cp}"),
        w.pending_url.clone(),
        "/v1/identities".to_string(),
        format!("/v1/sessions/{}/messages", w.client_id),
        format!("/v1/sessions/{}/messages", w.client_sid),
        "/v1/stats/global".to_string(),
        w.daily_url.clone(),
        w.tool_calls_url.clone(),
    ];
    assert_lab_view_unchanged_by_deleting_client_rows(&w, &urls).await;
}

/// stats-read-api D3 (a): the stats-read routes give a lab reader lab sessions
/// only in OBSERVE as well, where every other route still shows everything.
#[tokio::test]
async fn stats_read_routes_are_lab_only_in_observe_too() {
    let w = world_with(false).await;
    let urls = vec![w.daily_url.clone(), w.tool_calls_url.clone()];
    // The control: in observe a client reader's view does contain the client
    // session, so the lab view below is filtered, not merely empty.
    for url in &urls {
        let lab = get(&w, url, "lab").await;
        let client = get(&w, url, "client").await;
        assert_eq!((lab.0, client.0), (200, 200), "{lab:?} {client:?}");
        assert_ne!(
            lab.1, client.1,
            "observe: the client reader sees more on {url}"
        );
    }
    assert_lab_view_unchanged_by_deleting_client_rows(&w, &urls).await;
}

async fn assert_lab_view_unchanged_by_deleting_client_rows(w: &World, urls: &[String]) {
    async fn view(
        w: &World,
        urls: &[String],
        who: &str,
    ) -> Vec<(String, u16, Option<String>, String)> {
        let mut out = Vec::new();
        let as_of = regex::Regex::new(r#""as_of":"[^"]*""#).unwrap();
        for url in urls {
            let mut req = reqwest::Client::new().get(format!("{}{url}", w.base));
            req = match who {
                "lab" => req.bearer_auth(&w.lab_reader),
                _ => req.header("Tailscale-User-Login", LOGIN),
            };
            let r = req.send().await.unwrap();
            let total = r
                .headers()
                .get("x-total-count")
                .map(|v| v.to_str().unwrap().to_string());
            let status = r.status().as_u16();
            // `as_of` is the database snapshot each pending call takes: it
            // differs per request whatever the data, so it is masked.
            let body = r.text().await.unwrap();
            let body = as_of.replace_all(&body, r#""as_of":"-""#).into_owned();
            out.push((url.clone(), status, total, body));
        }
        out
    }
    let before_lab = view(w, urls, "lab").await;
    let before_ts = view(w, urls, "tailscale").await;

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    for sql in [
        "DELETE FROM journal_entries WHERE project_path = $1",
        "DELETE FROM project_identity_aliases WHERE project_path = $1",
        "DELETE FROM messages WHERE session_id IN (SELECT s.id FROM sessions s JOIN projects p ON p.id = s.project_id WHERE p.project_path = $1)",
        "DELETE FROM sessions WHERE project_id IN (SELECT id FROM projects WHERE project_path = $1)",
        "DELETE FROM projects WHERE project_path = $1",
    ] {
        sqlx::query(sql).bind(&w.client_path).execute(&pool).await.unwrap();
    }
    // The mirror folds in inserts only: rebuild it so the stats routes answer
    // from the archive without the client rows too.
    hub::mirror::rebuild(&w.mirror_cfg, &pool).await.unwrap();
    assert!(
        w.mirror.adopt_replacement().unwrap(),
        "rebuilt mirror adopted"
    );

    for (who, before) in [("lab", before_lab), ("tailscale", before_ts)] {
        let after = view(w, urls, who).await;
        for (b, a) in before.iter().zip(&after) {
            assert_eq!(b, a, "{who}: {} differs once the client rows are gone", b.0);
        }
    }
}

/// R2-H: a browser trades its token once for an `HttpOnly` cookie of the same
/// scope; a forged or signed-out cookie reads nothing.
#[tokio::test]
async fn login_cookie_carries_the_tokens_scope_and_nothing_more() {
    let w = world().await;
    let login = |token: String| {
        let base = w.base.clone();
        async move {
            let r = reqwest::Client::new()
                .post(format!("{base}/v1/login"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status().as_u16(), 204);
            let set = r
                .headers()
                .get("set-cookie")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            assert!(
                set.contains("HttpOnly")
                    && set.contains("Secure")
                    && set.contains("SameSite=Strict")
            );
            set.split(';').next().unwrap().to_string()
        }
    };
    let with_cookie = |cookie: String, url: String| {
        let base = w.base.clone();
        async move {
            let r = reqwest::Client::new()
                .get(format!("{base}{url}"))
                .header("Cookie", cookie)
                .send()
                .await
                .unwrap();
            (r.status().as_u16(), r.text().await.unwrap())
        }
    };
    let client_msgs = format!("/v1/sessions/{}/messages", w.client_id);

    let client_cookie = login(w.client_reader.clone()).await;
    let (status, body) = with_cookie(client_cookie.clone(), client_msgs.clone()).await;
    assert_eq!(status, 200);
    assert!(
        body.contains(&w.client_canary),
        "a client token's cookie reads client sessions"
    );

    let lab_cookie = login(w.lab_reader.clone()).await;
    let (status, _) = with_cookie(lab_cookie, client_msgs.clone()).await;
    assert_eq!(status, 404, "a lab token's cookie stays lab");

    let (status, _) = with_cookie("__Host-cchv_session=forged".into(), client_msgs.clone()).await;
    assert_eq!(status, 401);

    let r = reqwest::Client::new()
        .delete(format!("{}/v1/login", w.base))
        .header("Cookie", client_cookie.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 204);
    let (status, _) = with_cookie(client_cookie, client_msgs).await;
    assert_eq!(status, 401, "signed out");

    // A login cookie only reads: alias writes refuse it.
    let lab_cookie = login(w.lab_reader.clone()).await;
    let r = reqwest::Client::new()
        .post(format!("{}/v1/identities/aliases", w.base))
        .header("Cookie", lab_cookie)
        .json(&json!({"project_path": w.lab_path, "identity_key": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403, "a cookie cannot write aliases");

    // Only a bearer token logs in: not the Tailscale identity.
    let r = reqwest::Client::new()
        .post(format!("{}/v1/login", w.base))
        .header("Tailscale-User-Login", LOGIN)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
}

/// Security O1: every predicate is total. An unscanned session (NULL class or
/// generation) and a route-less entry are hidden from lab readers under NOT as
/// well as under WHERE: entries, journal search, threads, pending and the
/// health detail. The lab canary stays visible (positive control).
#[tokio::test]
async fn unscanned_sessions_and_route_less_entries_are_hidden() {
    let w = world().await;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let r = Uuid::new_v4().simple().to_string();
    let (unscanned, routeless) = (format!("unscannedcanary{r}"), format!("routelesscanary{r}"));
    let two_days_ago = (chrono::Utc::now() - chrono::Duration::days(2))
        .format("%Y-%m-%dT12:00:00Z")
        .to_string();
    // Three lab-looking sessions in three new projects.
    let mut ids = Vec::new();
    for (tag, ts) in [
        ("u", "2026-10-29T12:00:00Z"),
        ("n", "2026-10-29T12:00:00Z"),
        ("p", two_days_ago.as_str()),
    ] {
        let (path, sid) = (format!("/w/o1-{tag}-{r}"), Uuid::new_v4().to_string());
        let resp = reqwest::Client::new()
            .post(format!("{}/v1/ingest", w.base))
            .bearer_auth(&w.machine_token)
            .json(&json!({
                "machine": {"machine_id": w.machine, "hostname": format!("rt-{}", &r[..8]), "os": "macos"},
                "projects": [{"provider": "claude", "project_path": path, "name": "p"}],
                "sessions": [{"provider": "claude", "session_id": sid, "project_path": path}],
                "messages": [{"provider": "claude", "session_id": sid, "message_key": Uuid::new_v4().to_string(),
                              "seq": 0, "timestamp": ts, "type": "user", "role": "user",
                              "content": [{"type": "text", "text": "lab work"}], "raw": {"cwd": path}}],
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let id: i64 = sqlx::query_scalar("SELECT id FROM sessions WHERE session_id = $1")
            .bind(&sid)
            .fetch_one(&pool)
            .await
            .unwrap();
        ids.push((path, id));
    }
    // "u" and "p" become unscanned: NULL generation (as before any sweep).
    for k in [0, 2] {
        sqlx::query("UPDATE sessions SET client_gen = NULL WHERE id = $1")
            .bind(ids[k].1)
            .execute(&pool)
            .await
            .unwrap();
    }
    // "u": a GENERAL entry over the unscanned session. "n": a ROUTE-LESS entry.
    for (k, canary, route) in [(0, &unscanned, Some("general")), (1, &routeless, None)] {
        let mut body = json!({
            "entry_date": "2026-10-29", "project_path": ids[k].0, "status": "entry",
            "headline": format!("entry {canary}"), "summary": format!("summary {canary}"),
            "topics": ["one", "two", "three"], "open_questions": [format!("question {canary}")],
            "session_ids": [ids[k].1], "model": "m",
        });
        if let Some(route) = route {
            body["route"] = json!(route);
        }
        let resp = reqwest::Client::new()
            .post(format!("{}/v1/journal/entries", w.base))
            .bearer_auth(&w.machine_token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
    }
    let hidden_path = &ids[2].0;
    let mut leaks = Vec::new();
    for url in [
        "/v1/journal/entries?from=2026-10-01".to_string(),
        format!("/v1/search?q={unscanned}&scope=journal"),
        format!("/v1/search?q={routeless}&scope=journal"),
        "/v1/journal/open-questions?days=3650".to_string(),
        format!(
            "/v1/journal/open-questions?days=3650&project={}",
            urlencode(&ids[1].0)
        ),
        w.pending_url.clone(),
        "/v1/healthz/journal?within_days=7".to_string(),
    ] {
        for who in ["lab", "tailscale"] {
            let (status, body) = get(&w, &url, who).await;
            if body.contains(&unscanned)
                || body.contains(&routeless)
                || body.contains(hidden_path.as_str())
            {
                leaks.push(format!("{who} {url} -> {status}"));
            }
        }
    }
    assert!(
        leaks.is_empty(),
        "unscanned / route-less material served to lab readers: {leaks:?}"
    );
    let (_, body) = get(
        &w,
        &format!("/v1/journal/entries?project={}", urlencode(&w.lab_path)),
        "lab",
    )
    .await;
    assert!(
        body.contains(&w.lab_canary),
        "positive control: the stamped lab entry is still served"
    );
}

/// E8 (design R3-E8): the restamp's classify step refuses to run as a lab
/// caller under enforce, so the client-scope health names the caller's scope.
/// Anonymous callers get `null`; nobody gets more than their own scope.
#[tokio::test]
async fn client_scope_health_names_the_callers_scope() {
    let w = world().await;
    let mut seen = Vec::new();
    for who in ["client", "lab", "tailscale", "none"] {
        let (_, body) = get(&w, "/v1/healthz/client-scope", who).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        seen.push(format!("{who}={}", v["caller_scope"]));
    }
    assert_eq!(
        seen,
        [
            "client=\"client\"",
            "lab=\"lab\"",
            "tailscale=\"lab\"",
            "none=null"
        ]
    );
}

/// Security T1: client-only ingest, on a machine of its own AND on the machine
/// that also carries the lab session, leaves every stats-read response a lab
/// reader gets byte-identical, freshness fields included.
#[tokio::test]
async fn client_only_ingest_moves_no_stats_read_answer() {
    let w = world().await;
    let urls = [w.daily_url.clone(), w.tool_calls_url.clone()];
    let view = |w: &World| {
        let urls = urls.clone();
        let base = w.base.clone();
        let token = w.lab_reader.clone();
        async move {
            let mut out = Vec::new();
            for url in urls {
                let r = reqwest::Client::new()
                    .get(format!("{base}{url}"))
                    .bearer_auth(&token)
                    .send()
                    .await
                    .unwrap();
                out.push((url, r.status().as_u16(), r.text().await.unwrap()));
            }
            out
        }
    };
    let before = view(&w).await;
    assert!(before.iter().all(|(_, s, _)| *s == 200), "{before:?}");

    let now = chrono::Utc::now().to_rfc3339();
    let tool = format!(
        "mcp__exa__{}",
        w.tool_calls_url
            .split("prefix=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
    );
    let shared_host = format!("rt-{}", tool.rsplit("rt_").next().unwrap());
    for (machine, token, host, hostname) in [
        (w.machine, &w.machine_token, "shared", shared_host.clone()),
        (
            w.client_machine,
            &w.client_machine_token,
            "dedicated",
            "t1-dedicated".to_string(),
        ),
    ] {
        let sid = Uuid::new_v4().to_string();
        let path = format!("{}-{host}", w.client_path);
        let resp = reqwest::Client::new()
            .post(format!("{}/v1/ingest", w.base))
            .bearer_auth(token)
            .json(&json!({
                // Each machine sends its own name, as a real Mac does: the
                // shared one the name its lab ingest used.
                "machine": {"machine_id": machine, "hostname": hostname, "os": "macos"},
                "projects": [{"provider": "claude", "project_path": path, "name": "p"}],
                "sessions": [{"provider": "claude", "session_id": sid, "project_path": path, "summary": "s"}],
                "messages": [{"provider": "claude", "session_id": sid, "message_key": Uuid::new_v4().to_string(),
                              "seq": 0, "timestamp": now, "type": "assistant", "role": "assistant",
                              "content": [{"type": "tool_use", "id": "t1", "name": tool, "input": {}}],
                              "raw": {"cwd": path}}],
            }))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "{host} ingest");
    }
    w.mirror.refresh(&w.state.pool).await.unwrap();

    let after = view(&w).await;
    assert_eq!(
        before, after,
        "client-only ingest changed a lab stats-read answer"
    );
    // The control: a client reader does see the new calls.
    let (_, client_body) = get(&w, &w.tool_calls_url, "client").await;
    assert!(client_body.contains("\"count\":4"), "{client_body}");
}

/// Security T2: the client-flag check and the aggregation use one mirror
/// handle. A snapshot checked before an older mirror is adopted keeps
/// aggregating on what it checked; a request after the adoption refuses.
#[tokio::test]
async fn the_flag_check_and_the_rollup_share_one_mirror_handle() {
    let w = world().await;
    let vis = hub::auth::Visibility::stats_read(&w.state, hub::auth::ReadScope::Lab).unwrap();
    let window = || hub::stats::Window {
        from: Some(chrono::Utc::now().date_naive() - chrono::Duration::days(30)),
        to: Some(chrono::Utc::now().date_naive()),
        tz: chrono_tz::UTC,
        sessions_visible: None,
    };
    let checked = hub::stats_api::MirrorSnapshot::take(&w.state).unwrap();
    let checked_flags = checked.flags_as_of();
    let before_rows = {
        let wn = hub::stats_api::restrict(&w.state, vis.clone(), window(), checked.flags_as_of())
            .await
            .expect("current mirror passes the check");
        let snap = hub::stats_api::MirrorSnapshot::take(&w.state).unwrap();
        snap.run(move |c| Ok(hub::stats::daily_by_project(c, &wn)?))
            .await
            .unwrap()
            .1
    };

    // An older mirror: built now, then made to look like one copied before
    // the client sessions were raised (every class `none`, flags from long
    // ago), and swapped in beneath the live hub as `hub mirror rebuild` does.
    let older = w.mirror_cfg.resolved_path().with_extension("older");
    {
        let m = hub::mirror::Mirror::open_or_create(&hub::config::MirrorConfig {
            path: Some(older.clone()),
            ..hub::config::MirrorConfig::default()
        })
        .unwrap();
        m.refresh(&w.state.pool).await.unwrap();
        let c = m.connection().unwrap();
        c.execute_batch(
            "UPDATE mirror_meta SET v = '2000-01-01 00:00:00+00' WHERE k = 'client_flags_as_of';
             UPDATE sessions SET client_class = 'none'; CHECKPOINT;",
        )
        .unwrap();
    }
    std::fs::rename(&older, w.mirror_cfg.resolved_path()).unwrap();
    assert!(
        w.mirror.adopt_replacement().unwrap(),
        "older mirror adopted"
    );

    // The snapshot taken before the swap still aggregates on what it checked.
    let wn = hub::stats_api::restrict(&w.state, vis.clone(), window(), checked.flags_as_of())
        .await
        .expect("its own watermark is still current");
    let rows = checked
        .run(move |c| Ok(hub::stats::daily_by_project(c, &wn)?))
        .await
        .unwrap()
        .1;
    assert_eq!(rows, before_rows, "the checked snapshot's answer");

    // The control: aggregating on the swapped-in handle with the old check's
    // window would include the client session (the race T2 closes).
    let leaky = hub::stats_api::MirrorSnapshot::take(&w.state).unwrap();
    let wn = hub::stats_api::restrict(&w.state, vis.clone(), window(), checked_flags.clone())
        .await
        .unwrap();
    let leaked = leaky
        .run(move |c| Ok(hub::stats::daily_by_project(c, &wn)?))
        .await
        .unwrap()
        .1;
    assert_ne!(
        leaked, before_rows,
        "the older mirror would leak if aggregated on"
    );

    // A request after the swap sees the older watermark and refuses.
    let fresh = hub::stats_api::MirrorSnapshot::take(&w.state).unwrap();
    let refused = hub::stats_api::restrict(&w.state, vis, window(), fresh.flags_as_of()).await;
    assert!(
        matches!(refused, Err(hub::error::HubError::MirrorBehind(_))),
        "{refused:?}"
    );
    let (status, _) = get(&w, &w.daily_url, "lab").await;
    assert_eq!(status, 503, "over HTTP too");
}

/// The daily project name comes only from project rows the reader may see,
/// even when a client project row shares the identity and would win the name
/// (security, 2026-11-03, on afd762c9).
#[tokio::test]
async fn a_lab_reader_never_gets_a_client_projects_name() {
    let w = world_with(false).await;
    let pool = w.state.pool.clone();
    let key = format!("idk-{}", Uuid::new_v4().simple());
    // Sorts first, so on a tie of counts it would be the name.
    let client_name = format!("aaaclient-{}", Uuid::new_v4().simple());
    sqlx::query("UPDATE projects SET identity_key = $1 WHERE project_path = ANY($2)")
        .bind(&key)
        .bind(vec![w.lab_path.clone(), w.client_path.clone()])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE projects SET name = $1 WHERE project_path = $2")
        .bind(&client_name)
        .bind(&w.client_path)
        .execute(&pool)
        .await
        .unwrap();
    w.mirror.refresh(&pool).await.unwrap();
    // The control: a client reader is named by the client row.
    let (status, client) = get(&w, &w.daily_url, "client").await;
    assert_eq!(status, 200);
    assert!(client.contains(&client_name), "{client}");
    for who in ["lab", "stats"] {
        let (status, lab) = get(&w, &w.daily_url, who).await;
        assert_eq!(status, 200, "{who}");
        assert!(!lab.contains(&client_name), "{who}: {lab}");
        assert!(
            lab.contains("\"project\""),
            "{who}: the lab row is still there: {lab}"
        );
    }
}

/// A caller that skipped `restrict` is refused, never answered over every
/// session.
#[test]
fn daily_by_project_refuses_an_unset_visibility() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let w = hub::stats::Window::default();
    assert!(hub::stats::daily_by_project(&conn, &w).is_err());
}

/// openspec stats-stale-after-generation: stamp one session's coverage, as a
/// generation change (another generation) or a sweep (the live one) would.
async fn stamp(w: &World, id: i64, generation: &str) {
    sqlx::query("UPDATE sessions SET client_gen = $2 WHERE id = $1")
        .bind(id)
        .bind(generation)
        .execute(&w.state.pool)
        .await
        .unwrap();
}

async fn live_generation(w: &World) -> String {
    sqlx::query_scalar("SELECT generation FROM client_scope_state")
        .fetch_one(&w.state.pool)
        .await
        .unwrap()
}

fn unscanned_excluded(body: &str) -> i64 {
    serde_json::from_str::<Value>(body).unwrap()["unscanned_excluded"]
        .as_i64()
        .unwrap_or_else(|| panic!("no unscanned_excluded: {body}"))
}

/// Tasks 1.1, 1.2 and 1.6: a lab session last written before the margin and
/// unscanned under the live generation makes both lab stats routes 503 (never
/// a 200 without it); once a sweep stamps it, they answer 200 again.
#[tokio::test]
async fn a_generation_change_without_a_sweep_is_stale_never_empty() {
    let w = world_with(false).await;
    let live = live_generation(&w).await;
    stamp(&w, w.lab_id, "an-older-generation").await;
    for url in [&w.daily_url, &w.tool_calls_url] {
        for who in ["lab", "stats"] {
            let (status, body) = get(&w, url, who).await;
            assert_eq!(status, 503, "{who} {url}: {body}");
            assert!(
                body.contains("unscanned") && body.contains(&live),
                "{who} {url}: the reason names the count and the generation: {body}"
            );
        }
    }
    stamp(&w, w.lab_id, &live).await;
    w.mirror.refresh(&w.state.pool).await.unwrap();
    for url in [&w.daily_url, &w.tool_calls_url] {
        let (status, body) = get(&w, url, "lab").await;
        assert_eq!(status, 200, "{url}: {body}");
        assert_eq!(unscanned_excluded(&body), 0, "{url}");
    }
    let (_, daily) = get(&w, &w.daily_url, "lab").await;
    assert!(
        daily.contains("\"project\""),
        "the lab row is back: {daily}"
    );
}

/// Task 1.3: a lab session still being written (inside the margin) is left
/// out and counted, not a 503.
#[tokio::test]
async fn a_session_still_being_written_is_left_out_and_counted() {
    let w = world_with(false).await;
    stamp(&w, w.lab_id, "an-older-generation").await;
    sqlx::query(
        "UPDATE sessions SET last_message_time = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(w.lab_id)
    .execute(&w.state.pool)
    .await
    .unwrap();
    for url in [&w.daily_url, &w.tool_calls_url] {
        let (status, body) = get(&w, url, "lab").await;
        assert_eq!(status, 200, "{url}: {body}");
        assert_eq!(unscanned_excluded(&body), 1, "{url}");
    }
}

/// Tasks 1.4 and 1.5: an unscanned client-class session cannot make lab
/// counts incomplete, and a full-scope reader sees unscanned sessions.
#[tokio::test]
async fn unscanned_client_sessions_and_full_scope_readers_are_not_stale() {
    let w = world_with(false).await;
    stamp(&w, w.client_id, "an-older-generation").await;
    for url in [&w.daily_url, &w.tool_calls_url] {
        let (status, body) = get(&w, url, "lab").await;
        assert_eq!(status, 200, "{url}: {body}");
        assert_eq!(unscanned_excluded(&body), 0, "{url}");
    }
    stamp(&w, w.lab_id, "an-older-generation").await;
    for url in [&w.daily_url, &w.tool_calls_url] {
        let (status, body) = get(&w, url, "client").await;
        assert_eq!(status, 200, "{url}: {body}");
        assert_eq!(unscanned_excluded(&body), 0, "{url}");
    }
}

/// Security c9d749e (P3): client scope is configured but has no live
/// generation (the routing file cannot be read), so every session filters to
/// false for a lab reader. Both stats routes answer 503, never 200 with empty
/// rows.
#[tokio::test]
async fn no_live_generation_is_stale_never_empty() {
    let w = world_with(false).await;
    std::fs::remove_file(w.dir.join("r.toml")).unwrap();
    w.state
        .client_scope
        .as_ref()
        .unwrap()
        .refresh(&w.state.pool)
        .await
        .unwrap();
    for url in [&w.daily_url, &w.tool_calls_url] {
        for who in ["lab", "stats"] {
            let (status, body) = get(&w, url, who).await;
            assert_eq!(status, 503, "{who} {url}: {body}");
            assert!(body.contains("no live generation"), "{who} {url}: {body}");
        }
    }
    let (status, _) = get(&w, &w.daily_url, "client").await;
    assert_eq!(status, 200, "a full-scope reader still reads");
}
