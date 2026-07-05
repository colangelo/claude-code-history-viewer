//! Central archive hub library.
//!
//! The only component that holds Postgres credentials. Exposes a bearer-authed
//! ingest endpoint and (in later groups) search/browse. The router and migrator
//! are public so integration tests can drive them against a throwaway database.

pub mod auth;
pub mod browse;
pub mod config;
pub mod error;
pub mod health;
pub mod ingest;
pub mod pagination;
pub mod search;
pub mod state;

use axum::extract::DefaultBodyLimit;
use axum::http::header::AUTHORIZATION;
use axum::http::HeaderName;
use axum::routing::{get, post};
use axum::Router;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};

pub use config::HubConfig;
pub use state::AppState;

/// Embedded schema migrations (repo-root `migrations/`), shared by `run()` and tests.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Max request-body size. Axum's 2 MiB default rejects (413) real ingest
/// batches — a single agent message with a large tool result can exceed it on
/// its own, permanently blocking that session's sync. The hub is tailnet-only
/// and bearer-authed, so a generous cap is safe; the daemon's
/// `batch_max_messages` bounds typical batches well below this.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Build the HTTP router for the given state.
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
pub fn router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers([AUTHORIZATION])
        .expose_headers([HeaderName::from_static("x-total-count")]);

    Router::new()
        .route("/v1/healthz", get(health::healthz))
        .route("/v1/healthz/ingest", get(health::healthz_ingest))
        .route("/v1/ingest", post(ingest::ingest))
        .route("/v1/search", get(search::search))
        .route("/v1/projects", get(browse::list_projects))
        .route("/v1/sessions", get(browse::list_sessions))
        .route("/v1/sessions/{id}/messages", get(browse::session_messages))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(cors)
        .with_state(state)
}

/// Load config, connect to Postgres, apply migrations, and serve until shutdown.
pub async fn run() -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;
    MIGRATOR.run(&pool).await?;

    let state = AppState::new(pool, config.token_map());
    let app = router(state);

    let listener = TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "hub listening");
    axum::serve(listener, app).await?;
    Ok(())
}
