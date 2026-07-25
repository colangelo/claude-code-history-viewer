//! Central archive hub library.
//!
//! The only component that holds Postgres credentials. Exposes a bearer-authed
//! ingest endpoint and (in later groups) search/browse. The router and migrator
//! are public so integration tests can drive them against a throwaway database.

pub mod auth;
pub mod backfill;
pub mod browse;
pub mod config;
pub mod db_watchdog;
pub mod embed;
pub mod embed_sweep;
pub mod error;
pub mod extract;
pub mod fts;
pub mod health;
pub mod identities;
pub mod identity_filter;
pub mod ingest;
pub mod journal;
pub mod mirror;
pub mod pagination;
pub mod search;
pub mod state;
pub mod stats;
pub mod stats_api;
pub mod tz_spans;

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
        .expose_headers([
            HeaderName::from_static("x-total-count"),
            // Mirror staleness on `/v1/stats/*` (design D5). Same rule as
            // `X-Total-Count`: a header a script may read has to be exposed
            // explicitly, and the webapp calling the hub cross-origin is the
            // only consumer these exist for.
            HeaderName::from_static("x-stats-mirror-refreshed-at"),
            HeaderName::from_static("x-stats-mirror-age-seconds"),
        ]);

    let mut router = Router::new()
        .route("/v1/healthz", get(health::healthz))
        .route("/v1/healthz/ingest", get(health::healthz_ingest))
        .route("/v1/healthz/journal", get(health::healthz_journal))
        .route("/v1/healthz/stats", get(health::healthz_stats))
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
        .route("/v1/stats/global", get(stats_api::global))
        .route("/v1/stats/projects/{identity_key}", get(stats_api::project))
        .route("/v1/stats/sessions/{id}", get(stats_api::session))
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
        // API responses carry NO freshness metadata of their own, which lets a
        // browser heuristically cache a `/v1/journal/entries` (or search/browse)
        // GET and keep serving a stale copy across reloads — the "I refreshed
        // and the journal still doesn't show yesterday" bug. `no-store` forbids
        // caching outright so every read hits the hub. `if_not_present` leaves
        // the static block's own `Cache-Control` untouched (assets stay
        // `immutable`, `index.html` stays `no-cache`) — this outermost layer
        // runs last on the response, after those inner layers have set theirs,
        // so it only fills the gap on the `/v1/*` API surface.
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .with_state(state)
}

/// Apply pending migrations and exit.
///
/// Serving and the backfill both apply migrations on startup, so this exists
/// purely to make the DDL a *separate, observable step* during a deploy —
/// schema first, verify, then touch data. Rolling one into the other is how you
/// end up unable to say which half of an operation failed.
pub async fn run_migrate() -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await?;
    MIGRATOR.run(&pool).await?;
    tracing::info!("migrations applied");
    Ok(())
}

/// Load config, connect, and run the analytics backfill to completion.
///
/// A separate entry point rather than a startup sweep: this is a one-time
/// catch-up over the whole archive, not a steady-state reconciliation, so it
/// should be run deliberately and watched — not fired on every hub boot where it
/// would compete with serving traffic. Migrations are applied first so the
/// target columns are guaranteed to exist.
pub async fn run_backfill(batch: i64) -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await?;
    MIGRATOR.run(&pool).await?;

    let stats = backfill::run(&pool, batch).await?;
    tracing::info!(
        scanned = stats.scanned,
        message_ids = stats.message_ids,
        tool_uses = stats.tool_uses,
        tool_results = stats.tool_results,
        "analytics backfill finished"
    );
    tracing::warn!(
        "backfill rewrote `message_id` on existing rows — run `hub mirror rebuild` \
         or the statistics mirror will keep those rows in their old dedup groups \
         and over-count tokens"
    );
    Ok(())
}

/// Rebuild the statistics mirror from scratch and swap it in.
///
/// Deliberately a separate, watched operation rather than a startup sweep: it
/// is the counterpart to `backfill-analytics` (design D2), not steady state.
/// The running hub keeps serving from the old mirror while this builds.
pub async fn run_mirror_rebuild() -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await?;
    let report = mirror::rebuild(&config.stats_mirror, &pool).await?;
    tracing::info!(
        messages = report.messages_inserted,
        tool_uses = report.tool_uses_inserted,
        tool_results = report.tool_results_inserted,
        elapsed_s = report.elapsed.as_secs(),
        "mirror rebuild finished"
    );
    Ok(())
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
        // Startup + interval + nudge-driven embedding sweep (bootstrap of
        // pre-existing entries is just the first pass).
        tokio::spawn(embed_sweep::run_sweeper(
            state.clone(),
            std::time::Duration::from_secs(300),
        ));
    }

    // Statistics read model. Opening it is not allowed to take the hub down:
    // ingest, search, browse and the journal are unaffected by a mirror fault,
    // and `/v1/stats/*` already has a defined answer for "no mirror" (503).
    match mirror::Mirror::open_or_create(&config.stats_mirror) {
        Ok(m) => {
            let m = std::sync::Arc::new(m);
            if m.is_empty() {
                tracing::info!(
                    "stats mirror empty — /v1/stats/* answers 503 until the first build finishes"
                );
            }
            state = state.with_mirror(m.clone());
            tokio::spawn(mirror::run_refresher(
                m,
                state.pool.clone(),
                std::time::Duration::from_secs(config.stats_mirror.refresh_secs),
            ));
        }
        Err(e) => {
            tracing::error!(error = %e, "stats mirror unavailable; /v1/stats/* will answer 503");
        }
    }

    if let Some(dir) = &config.static_dir {
        tracing::info!(dir = %dir.display(), "serving static archive webapp at /");
    }
    let app = router(state, config.static_dir.as_deref());

    // Credential watchdog: our Postgres password is bao-owned and rotates on a
    // 30-day period, but it is resolved once, here, at startup. Exiting on a
    // sustained rejection is the whole recovery mechanism — launchd `KeepAlive`
    // relaunches us through `cchv-launch`, which re-reads the rotated credential.
    // Only SQLSTATE 28P01 counts, so a pg1 outage or DNS flake can never trip it.
    let db_fatal = std::sync::Arc::new(tokio::sync::Notify::new());
    tokio::spawn(db_watchdog::run_watchdog(
        config.database_url.clone(),
        db_fatal.clone(),
        db_watchdog::DEFAULT_INTERVAL,
        db_watchdog::DEFAULT_STRIKE_LIMIT,
    ));

    let listener = TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "hub listening");

    // The exit decision belongs to whoever owns the server's lifetime, so the
    // watchdog only signals and we translate that into a non-zero exit here.
    tokio::select! {
        served = axum::serve(listener, app) => served?,
        () = db_fatal.notified() => {
            anyhow::bail!(
                "postgres rejected the hub's credential on {} consecutive probes — \
                 exiting so the supervisor re-resolves it",
                db_watchdog::DEFAULT_STRIKE_LIMIT
            );
        }
    }
    Ok(())
}
