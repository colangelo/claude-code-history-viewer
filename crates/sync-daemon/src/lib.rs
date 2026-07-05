//! Per-machine sync daemon.
//!
//! Backfills all local agent history into the hub, then keeps it current by
//! rescanning on an interval. Holds only a hub URL + bearer token (no database
//! credentials). The archive is cumulative: deleting a local file never deletes
//! anything from the hub.

pub mod checkpoint;
pub mod client;
pub mod config;
pub mod convert;
pub mod fs_atomic;
pub mod identity;
pub mod sync;
pub mod watcher;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::checkpoint::Checkpoint;
use crate::client::ReqwestHubClient;
use crate::config::DaemonConfig;
use crate::identity::Identity;
use crate::watcher::PassThrottle;

/// Everything a sync pass needs, resolved from config + state dir.
struct Runtime {
    config: DaemonConfig,
    identity: Identity,
    client: ReqwestHubClient,
    checkpoint: Checkpoint,
    exclude: Vec<history_core::providers::ProviderId>,
}

impl Runtime {
    fn init() -> anyhow::Result<Self> {
        let config = DaemonConfig::load()?;
        let state_dir = config.resolve_state_dir()?;
        let identity = Identity::load_or_create(&state_dir)?;
        tracing::info!(machine_id = %identity.machine_id, hostname = %identity.hostname, "daemon identity");

        let client = ReqwestHubClient::new(&config.hub_url, &config.hub_token);
        let checkpoint = Checkpoint::load(&state_dir);

        let mut exclude = Vec::new();
        for id in &config.providers_exclude {
            match history_core::providers::ProviderId::parse(id) {
                Some(p) => exclude.push(p),
                None => {
                    tracing::warn!(provider = %id, "providers_exclude: unknown provider id, ignoring");
                }
            }
        }
        if !exclude.is_empty() {
            tracing::info!(?exclude, "provider scan exclusions active");
        }
        Ok(Runtime {
            config,
            identity,
            client,
            checkpoint,
            exclude,
        })
    }
}

/// Run exactly one sync pass and exit — no watcher, no rescan loop. Errors in
/// the pass surface as a non-zero exit so backfill scripts can gate on it.
pub async fn run_once() -> anyhow::Result<()> {
    let mut rt = Runtime::init()?;
    let stats = sync::run_once(
        &rt.client,
        &rt.identity,
        &mut rt.checkpoint,
        rt.config.batch_max_messages,
        &rt.exclude,
    )
    .await;
    tracing::info!(?stats, "sync pass complete");
    if stats.errors > 0 {
        anyhow::bail!("sync pass completed with {} error(s)", stats.errors);
    }
    Ok(())
}

/// Run the daemon until Ctrl-C: one immediate sync pass, then a pass every
/// `scan_interval_secs` (the safety-net rescan), plus early passes triggered
/// by the debounced file watcher (a latency optimization only — the rescan
/// keeps firing on its own schedule regardless of watcher activity).
pub async fn run() -> anyhow::Result<()> {
    let Runtime {
        config,
        identity,
        client,
        mut checkpoint,
        exclude,
    } = Runtime::init()?;

    // Watch-root discovery runs provider detection, which walks the
    // filesystem and can block indefinitely on cloud-backed dirs (observed
    // live: crush::detect wedged in an uncancellable opendir at startup, so
    // exclusion must happen before detect() runs, and the whole discovery
    // gets a deadline). On timeout the daemon stays rescan-only — watching
    // is a latency optimization, never worth a hang.
    let detect_exclude = exclude.clone();
    let watch_roots: Vec<PathBuf> = match tokio::time::timeout(
        Duration::from_secs(15),
        tokio::task::spawn_blocking(move || {
            history_core::providers::detect_providers_except(&detect_exclude)
        }),
    )
    .await
    {
        Ok(Ok(infos)) => infos
            .into_iter()
            .filter(|info| info.is_available)
            .map(|info| PathBuf::from(info.base_path))
            .collect(),
        Ok(Err(error)) => {
            tracing::warn!(%error, "provider detection for watch roots failed; continuing rescan-only");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!("provider detection for watch roots timed out; continuing rescan-only");
            Vec::new()
        }
    };

    let mut watcher_guard = None;
    let mut watcher_rx = None;
    match watcher::spawn(
        &watch_roots,
        Duration::from_secs(config.watch_debounce_secs),
    ) {
        Ok((guard, rx)) => {
            watcher_guard = Some(guard);
            watcher_rx = Some(rx);
        }
        Err(error) => {
            tracing::warn!(%error, "file watcher unavailable, continuing rescan-only");
        }
    }
    // Held only to keep the watcher alive for the daemon's lifetime; dropping
    // it would stop watching.
    let _watcher_guard = watcher_guard;

    let mut throttle = PassThrottle::new(Duration::from_secs(config.watch_min_pass_gap_secs));

    // The safety-net rescan runs on its own fixed schedule; only firing this
    // arm advances the deadline, so watcher-triggered early passes never
    // postpone it.
    let mut rescan_deadline =
        tokio::time::Instant::now() + Duration::from_secs(config.scan_interval_secs);

    loop {
        let stats = sync::run_once(
            &client,
            &identity,
            &mut checkpoint,
            config.batch_max_messages,
            &exclude,
        )
        .await;
        tracing::info!(?stats, "sync pass complete");
        throttle.note_pass(Instant::now());

        loop {
            // A pending trigger recorded mid-gap must be rechecked once
            // `min_gap` elapses even if no further filesystem activity
            // arrives to prompt it — otherwise it silently waits for the
            // next hourly rescan instead of producing the early pass.
            let pending_deadline = throttle
                .pending_due_at()
                .map(tokio::time::Instant::from_std);
            tokio::select! {
                () = tokio::time::sleep_until(rescan_deadline) => {
                    rescan_deadline = tokio::time::Instant::now() + Duration::from_secs(config.scan_interval_secs);
                    break;
                }
                () = sleep_until_opt(pending_deadline), if pending_deadline.is_some() => {
                    if throttle.pass_due(Instant::now()) {
                        break;
                    }
                }
                Some(()) = recv_watch_signal(&mut watcher_rx), if watcher_rx.is_some() => {
                    throttle.note_trigger(Instant::now());
                    if throttle.pass_due(Instant::now()) {
                        break;
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("received Ctrl-C, shutting down");
                    return Ok(());
                }
            }
        }
    }
}

/// Awaits the next watcher signal, or never resolves if no watcher is active.
async fn recv_watch_signal(rx: &mut Option<tokio::sync::mpsc::Receiver<()>>) -> Option<()> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Sleeps until `deadline`, or never resolves if there is no deadline to wait on.
async fn sleep_until_opt(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending().await,
    }
}
