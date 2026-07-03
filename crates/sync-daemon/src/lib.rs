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

use std::time::Duration;

use crate::checkpoint::Checkpoint;
use crate::client::ReqwestHubClient;
use crate::config::DaemonConfig;
use crate::identity::Identity;

/// Run the daemon until Ctrl-C: one immediate sync pass, then a pass every
/// `scan_interval_secs` (the safety-net rescan).
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

        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(config.scan_interval_secs)) => {}
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received Ctrl-C, shutting down");
                break;
            }
        }
    }
    Ok(())
}
