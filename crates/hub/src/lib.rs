//! Central archive hub library.
//!
//! The only component that holds Postgres credentials. Exposes a bearer-authed
//! ingest endpoint and (in later groups) search/browse. The router and migrator
//! are public so integration tests can drive them against a throwaway database.

pub mod auth;
pub mod backfill;
pub mod browse;
pub mod client_admin;
pub mod client_ingest;
pub mod client_policy;
pub mod client_scope;
pub mod client_state;
pub mod client_sweep;
pub mod config;
pub mod db_watchdog;
pub mod embed;
pub mod embed_sweep;
pub mod error;
pub mod extract;
pub mod findings;
pub mod fts;
pub mod health;
pub mod identities;
pub mod identity_filter;
pub mod ingest;
pub mod journal;
pub mod journal_threads;
pub mod login;
pub mod mirror;
pub mod pagination;
pub mod pair;
pub mod redact;
pub mod search;
pub mod state;
pub mod stats;
pub mod stats_api;
pub mod tz_spans;
pub mod visibility;

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
        .route("/v1/healthz/findings", get(findings::healthz))
        .route("/v1/healthz/client-scope", get(client_state::healthz))
        .route("/v1/ingest", post(ingest::ingest))
        .route("/v1/login", post(login::login).delete(login::logout))
        .route("/v1/findings/summary", get(findings::summary))
        .route("/v1/search", get(search::search))
        .route("/v1/journal/pending", get(journal::pending))
        .route(
            "/v1/journal/entries",
            get(journal::browse).post(journal::create),
        )
        .route("/v1/journal/ticks", post(journal::record_tick))
        .route(
            "/v1/journal/open-questions",
            get(journal_threads::open_questions),
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
        // Inside CORS, so a refused write still carries the CORS headers and a
        // browser shows the 401 instead of an opaque network error.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::refuse_read_only_writes,
        ))
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
/// `hub findings dry-run`: scan stored messages with the ingest detector and
/// print the report (Gitea #34, infra#104). Every session on the pool is
/// read-only, and no migration runs, so the command cannot write to the archive.
pub async fn run_findings_dry_run(opts: findings::DryRunOptions) -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET default_transaction_read_only = on")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("SET statement_timeout = '60s'")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&config.database_url)
        .await?;
    let report = findings::dry_run(&pool, &opts).await?;
    print!("{}", report.render());
    Ok(())
}

/// The audited client-scope actions (`hub client-scope accept-routing | clear |
/// unlatch | restamp-history`), as the pg1 writer. Each prints what it did,
/// counts only.
pub async fn run_client_scope_admin(action: ClientScopeAdmin) -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    let cs = config
        .client_scope
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no [client_scope] in the hub config"))?;
    let read_only = matches!(
        action,
        ClientScopeAdmin::Restamp {
            approved_by: None,
            ..
        }
    );
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .after_connect(move |conn, _meta| {
            Box::pin(async move {
                if read_only {
                    sqlx::query("SET default_transaction_read_only = on")
                        .execute(&mut *conn)
                        .await?;
                }
                Ok(())
            })
        })
        .connect(&config.database_url)
        .await?;
    let policy = || match client_policy::load(cs) {
        client_policy::Load::Ready(p) => Ok(p),
        client_policy::Load::Unknown(why) => {
            Err(anyhow::anyhow!("client scope policy unknown: {why}"))
        }
    };
    match action {
        ClientScopeAdmin::AcceptRouting { approved_by } => {
            let p = policy()?;
            client_admin::accept_routing(&pool, &p, &approved_by).await?;
            println!("accepted the routing generation ({} markers); the hub adopts it at its next refresh", p.marker_hashes.len());
        }
        ClientScopeAdmin::Clear {
            session,
            reason,
            approved_by,
        } => {
            client_admin::clear(&pool, session, &reason, &approved_by).await?;
            println!("cleared session {session}; it is lab only after a sweep stamps it");
        }
        ClientScopeAdmin::Unlatch {
            reason,
            approved_by,
        } => {
            client_admin::unlatch(&pool, &reason, &approved_by).await?;
            println!("enforcement latch lifted; a config with enforce = false now observes");
        }
        ClientScopeAdmin::Restamp { from, approved_by } => {
            let file = std::fs::read(&from)
                .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", from.display()))?;
            let r = client_admin::restamp_from_file(
                &pool,
                &file,
                approved_by.as_deref(),
                chrono::Utc::now(),
            )
            .await?;
            println!(
                "restamp-history{}: {}",
                if approved_by.is_some() {
                    ""
                } else {
                    " (dry run, nothing written)"
                },
                r.render()
            );
        }
    }
    Ok(())
}

/// One audited client-scope action, as parsed by the CLI.
pub enum ClientScopeAdmin {
    AcceptRouting {
        approved_by: String,
    },
    Clear {
        session: i64,
        reason: String,
        approved_by: String,
    },
    Unlatch {
        reason: String,
        approved_by: String,
    },
    /// Apply the distiller's `--restamp-history` file (design R3-E8);
    /// `approved_by: None` is the dry run.
    Restamp {
        from: std::path::PathBuf,
        approved_by: Option<String>,
    },
}

/// `hub client-scope sweep [--dry-run]`: classify every session not scanned
/// under the generation the hub has accepted (openspec `hub-client-read-scope`,
/// task 2.4). A dry run is read-only and prints counts only. Refuses when the
/// routing file is unusable or is not the generation the hub accepted.
pub async fn run_client_scope_sweep(dry_run: bool, batch: i64) -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    let cs = config
        .client_scope
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no [client_scope] in the hub config"))?;
    let policy = match client_policy::load(cs) {
        client_policy::Load::Ready(p) => p,
        client_policy::Load::Unknown(why) => anyhow::bail!("client scope policy unknown: {why}"),
    };
    let read_only = dry_run;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .after_connect(move |conn, _meta| {
            Box::pin(async move {
                if read_only {
                    sqlx::query("SET default_transaction_read_only = on")
                        .execute(&mut *conn)
                        .await?;
                }
                Ok(())
            })
        })
        .connect(&config.database_url)
        .await?;
    let accepted: Option<String> = sqlx::query_scalar("SELECT generation FROM client_scope_state")
        .fetch_optional(&pool)
        .await?
        .flatten();
    if accepted.as_deref() != Some(policy.generation.as_str()) {
        anyhow::bail!(
            "the hub has not accepted this routing generation yet \
             (start or wait for the hub, or run client-scope accept-routing)"
        );
    }
    if dry_run {
        // The dry run is the effective-class census (security O3): every
        // session, families closed, counts only.
        let c = client_sweep::census(&pool, &policy, None).await?;
        println!("client-scope census (dry run, nothing written)");
        println!("sessions: {}", c.sessions);
        println!("  client, own content: {}", c.local);
        println!("  client, family only: {}", c.family_only);
        println!(
            "  client, protected journal entry only: {}",
            c.protected_entry_only
        );
        println!("  lab: {}", c.lab);
        println!(
            "    of which the distiller's terms would protect (term-only): {}",
            c.term_only
        );
        return Ok(());
    }
    let report = client_sweep::sweep(
        &pool,
        &policy,
        &client_sweep::SweepOptions {
            dry_run,
            batch,
            only_sessions: None,
        },
    )
    .await?;
    println!(
        "client-scope sweep{}",
        if dry_run {
            " (dry run, nothing written)"
        } else {
            ""
        }
    );
    println!("sessions scanned: {}", report.scanned);
    for (class, n) in &report.by_class {
        println!("  class {class}: {n}");
    }
    println!(
        "term-only (left lab; the distiller's terms would protect): {}",
        report.term_only
    );
    println!(
        "family, protected journal entry: {}",
        report.family_protected_entry
    );
    println!("family, other members: {}", report.family_other);
    println!("not stamped (ingest raced; re-run): {}", report.not_stamped);
    Ok(())
}

/// `hub findings journal-scan`: the detector over the journal entries,
/// read-only, locations and rule counts only.
pub async fn run_findings_journal_scan(rules: Vec<redact::Rule>) -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET default_transaction_read_only = on")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&config.database_url)
        .await?;
    let (scanned, hits) = findings::journal_scan(&pool, &rules).await?;
    print!("{}", findings::render_journal_scan(scanned, &hits));
    Ok(())
}

/// `hub findings redact`: the real run (task 3.3). A writable pool, one
/// connection, so its row transactions run strictly one after another.
pub async fn run_findings_redact(opts: findings::RedactOptions) -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET statement_timeout = '60s'")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("SET lock_timeout = '10s'")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&config.database_url)
        .await?;
    let report = findings::redact_existing(&pool, &opts).await?;
    print!("{}", report.render());
    Ok(())
}

/// `hub findings ack`: acknowledge triaged findings so they stop paging
/// (Gitea #76). One transaction; refuses before writing on a bad selection.
pub async fn run_findings_ack(opts: findings::AckOptions) -> anyhow::Result<()> {
    let config = HubConfig::load()?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET statement_timeout = '60s'")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&config.database_url)
        .await?;
    let report = findings::ack(&pool, &opts).await?;
    print!("{}", report.render());
    Ok(())
}

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

    let state = AppState::from_config(pool, &config)?;
    tracing::info!(
        redact = ?state.redact_rules.iter().map(redact::Rule::id).collect::<Vec<_>>(),
        "credential detection: every rule flags; listed rules also redact"
    );
    let mut state = state.with_orphan_latch().await?;
    // Client scope: load the policy before serving, then re-validate it every
    // 60 s (ingest also re-validates before each batch). A failure to read the
    // database here is fatal: the hub would not know its latch.
    if let Some(scope) = state.client_scope.clone() {
        scope.refresh(&state.pool).await?;
        tracing::info!(
            enforcing = scope.enforcing(),
            ready = scope.snapshot().generation().is_some(),
            "client scope configured"
        );
        let pool = state.pool.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            tick.tick().await;
            loop {
                tick.tick().await;
                if let Err(e) = scope.refresh(&pool).await {
                    tracing::error!(error = %e, "client scope refresh failed; keeping the last state");
                }
            }
        });
    }
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
