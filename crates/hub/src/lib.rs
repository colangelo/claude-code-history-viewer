//! Central archive hub library.
//!
//! The only component that holds Postgres credentials. Exposes a bearer-authed
//! ingest endpoint and (in later groups) search/browse. The router and migrator
//! are public so integration tests can drive them against a throwaway database.

pub mod auth;
pub mod config;
pub mod error;
pub mod health;
pub mod ingest;
pub mod state;

use axum::routing::{get, post};
use axum::Router;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;

pub use config::HubConfig;
pub use state::AppState;

/// Embedded schema migrations (repo-root `migrations/`), shared by `run()` and tests.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Build the HTTP router for the given state.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/healthz", get(health::healthz))
        .route("/v1/ingest", post(ingest::ingest))
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
