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

/// Run the daemon until Ctrl-C: one immediate sync pass, then a pass every
/// `scan_interval_secs` (the safety-net rescan), plus early passes triggered
/// by the debounced file watcher (a latency optimization only — the rescan
/// keeps firing on its own schedule regardless of watcher activity).
pub async fn run() -> anyhow::Result<()> {
    let config = DaemonConfig::load()?;
    let state_dir = config.resolve_state_dir()?;
    let identity = Identity::load_or_create(&state_dir)?;
    tracing::info!(machine_id = %identity.machine_id, hostname = %identity.hostname, "daemon identity");

    let client = ReqwestHubClient::new(&config.hub_url, &config.hub_token);
    let mut checkpoint = Checkpoint::load(&state_dir);

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

    let watch_roots: Vec<PathBuf> = history_core::providers::detect_providers()
        .into_iter()
        .filter(|info| info.is_available)
        .filter_map(|info| {
            let provider = history_core::providers::ProviderId::parse(&info.id)?;
            (!exclude.contains(&provider)).then(|| PathBuf::from(info.base_path))
        })
        .collect();

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

        // Rescan on its own schedule regardless of watcher activity; a
        // throttled watcher trigger cuts this wait short for an early pass.
        let rescan_deadline =
            tokio::time::Instant::now() + Duration::from_secs(config.scan_interval_secs);
        loop {
            tokio::select! {
                () = tokio::time::sleep_until(rescan_deadline) => break,
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
