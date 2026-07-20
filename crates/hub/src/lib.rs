//! Central archive hub library.
//!
//! The only component that holds Postgres credentials. Exposes a bearer-authed
//! ingest endpoint and (in later groups) search/browse. The router and migrator
//! are public so integration tests can drive them against a throwaway database.

pub mod auth;
pub mod browse;
pub mod config;
pub mod embed;
pub mod error;
pub mod fts;
pub mod health;
pub mod identities;
pub mod identity_filter;
pub mod ingest;
pub mod journal;
pub mod pagination;
pub mod search;
pub mod state;

use axum::extract::DefaultBodyLimit;
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue};
use axum::routing::{delete, get, post};
use axum::Router;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use std::path::Path;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

pub use config::HubConfig;
pub use state::AppState;

/// Embedded schema migrations (repo-root `migrations/`), shared by `run()` and tests.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Max request-body size. Axum's 2 MiB default rejects (413) real ingest
/// batches — a single agent message with a large tool result can exceed it on
/// its own, permanently blocking that session's sync. Real transcripts hold
/// single 40 MiB records (Time Machine backfill, EMS-Roster 2026-01), which
/// serialize to ~2x that as an `IngestMessage` (raw + content) — hence 256 MiB.
/// The hub is tailnet-only and bearer-authed, so a generous cap is safe; the
/// daemon bounds typical batches by count AND bytes far below this.
const MAX_BODY_BYTES: usize = 256 * 1024 * 1024;

/// Build the HTTP router for the given state.
///
/// `static_dir`, when set, serves that directory at `/` (the static archive
/// webapp — see `openspec/specs/hub-static-hosting/spec.md`). It is wired as
/// the router *fallback*, so every explicitly registered `/v1/*` route wins
/// structurally — even over a `v1/` directory inside the static root. Static
/// assets are deliberately outside bearer auth (tailnet-only exposure; auth
/// guards the data endpoints, not the public bundle). Unset keeps axum's
/// plain 404 fallback, byte-identical to the pre-static behavior.
///
/// Cache policy follows the standard SPA split so a webapp rsync takes effect
/// on the next load with no hard reload: content-hashed `/assets/*` are
/// `immutable` (a new build changes the filename), while the `index.html`
/// entry point is `no-cache` — stored but always revalidated (`ServeDir` sends
/// `last-modified`, so an unchanged page still 304s). Without this, browsers
/// heuristically cache a `Cache-Control`-less `index.html` and keep loading a
/// stale hashed bundle after every update.
///
/// CORS allows any origin because the hub is tailnet-only and every read is
/// still gated by the bearer token; this layer only lifts the browser's
/// same-origin block so the viewer's webview/browser contexts can call the
/// hub directly (no viewer-side proxy — see
/// `openspec/specs/archive-search-api/spec.md`). `Authorization` must be
/// listed explicitly: per the Fetch spec a wildcard
/// `Access-Control-Allow-Headers: *` does NOT cover `Authorization`, so a
/// browser preflight would otherwise still block the bearer-token requests
/// this API requires. `X-Total-Count` similarly must be explicitly exposed
/// since it isn't on the CORS-safelisted response header list `fetch` allows
/// scripts to read by default.
pub fn router(state: AppState, static_dir: Option<&Path>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        // CONTENT_TYPE: the alias POST sends application/json, which is not
        // preflight-safelisted once the request also carries Authorization.
        .allow_headers([AUTHORIZATION, CONTENT_TYPE])
        .expose_headers([HeaderName::from_static("x-total-count")]);

    let mut router = Router::new()
        .route("/v1/healthz", get(health::healthz))
        .route("/v1/healthz/ingest", get(health::healthz_ingest))
        .route("/v1/ingest", post(ingest::ingest))
        .route("/v1/search", get(search::search))
        .route("/v1/journal/pending", get(journal::pending))
        .route(
            "/v1/journal/entries",
            get(journal::browse).post(journal::create),
        )
        .route("/v1/projects", get(browse::list_projects))
        .route("/v1/sessions", get(browse::list_sessions))
        .route("/v1/sessions/{id}/messages", get(browse::session_messages))
        .route("/v1/identities", get(identities::list))
        .route("/v1/identities/aliases", post(identities::create_alias))
        .route(
            "/v1/identities/aliases/{id}",
            delete(identities::delete_alias),
        );

    if let Some(dir) = static_dir {
        // Content-hashed assets: cache hard, never revalidate.
        let assets = ServiceBuilder::new()
            .layer(SetResponseHeaderLayer::overriding(
                CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ))
            .service(ServeDir::new(dir.join("assets")));
        // index.html (and any other top-level file): always revalidate.
        let root = ServiceBuilder::new()
            .layer(SetResponseHeaderLayer::overriding(
                CACHE_CONTROL,
                HeaderValue::from_static("no-cache"),
            ))
            .service(ServeDir::new(dir));
        router = router
            .nest_service("/assets", assets)
            .fallback_service(root);
    }

    router
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(cors)
        .with_state(state)
}

/// Load config, connect to Postgres, apply migrations, and serve until shutdown.
pub async fn run() -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    // Pool resilience (issue #17): keep warm connections so a transient DNS
    // flake (MagicDNS at 03:30) can't 500 every read — established conns need
    // no re-resolution; `test_before_acquire` (default on) pings them without
    // DNS. `acquire_timeout` fails fast instead of piling 30s waits. This is
    // mitigation, not cure: a flake outlasting the connection lifetime still
    // bites on the next reconnect.
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .min_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await?;
    MIGRATOR.run(&pool).await?;

    let mut state = AppState::new(
        pool,
        config.token_map(),
        config.trust_tailscale_identity.clone(),
    );
    if let Some(dir) = &config.embed_model_dir {
        tracing::info!(dir = %dir.display(), "embed model configured (lazy load)");
        state = state.with_embedder(std::sync::Arc::new(embed::CandleEmbedder::new(dir.clone())));
    }
    if let Some(dir) = &config.static_dir {
        tracing::info!(dir = %dir.display(), "serving static archive webapp at /");
    }
    let app = router(state, config.static_dir.as_deref());

    let listener = TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "hub listening");
    axum::serve(listener, app).await?;
    Ok(())
}
