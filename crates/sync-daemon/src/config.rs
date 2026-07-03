//! Daemon configuration: where the hub is and how to authenticate to it.
//! Intentionally holds NO database credentials.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonConfig {
    /// Base URL of the hub, e.g. `http://hub.tailnet:8787`.
    pub hub_url: String,
    /// Bearer token identifying this machine to the hub.
    pub hub_token: String,
    /// Seconds between safety-net rescans.
    #[serde(default = "default_scan_interval")]
    pub scan_interval_secs: u64,
    /// Max messages per ingest batch.
    #[serde(default = "default_batch_size")]
    pub batch_max_messages: usize,
    /// State directory (machine id + checkpoint). Defaults to `~/.claude-history-sync`.
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
    /// Provider ids to skip entirely (e.g. `["crush", "aider"]` on machines
    /// where their home-directory discovery walk is expensive). Unknown ids
    /// are logged and ignored at startup.
    #[serde(default)]
    pub providers_exclude: Vec<String>,
}

fn default_scan_interval() -> u64 {
    3600
}
fn default_batch_size() -> usize {
    500
}

impl DaemonConfig {
    /// Load from the TOML file at `DAEMON_CONFIG`, else from environment
    /// variables (`HUB_URL`, `HUB_TOKEN`, optional `SCAN_INTERVAL_SECS`).
    pub fn load() -> anyhow::Result<Self> {
        if let Ok(path) = std::env::var("DAEMON_CONFIG") {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("reading DAEMON_CONFIG {path}: {e}"))?;
            return toml::from_str(&text).map_err(|e| anyhow::anyhow!("parsing {path}: {e}"));
        }
        let hub_url = std::env::var("HUB_URL")
            .map_err(|_| anyhow::anyhow!("HUB_URL or DAEMON_CONFIG must be set"))?;
        let hub_token = std::env::var("HUB_TOKEN")
            .map_err(|_| anyhow::anyhow!("HUB_TOKEN or DAEMON_CONFIG must be set"))?;
        Ok(DaemonConfig {
            hub_url,
            hub_token,
            scan_interval_secs: std::env::var("SCAN_INTERVAL_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(default_scan_interval),
            batch_max_messages: default_batch_size(),
            state_dir: None,
            providers_exclude: Vec::new(),
        })
    }

    /// Resolve the state directory (`state_dir` override or `~/.claude-history-sync`).
    pub fn resolve_state_dir(&self) -> anyhow::Result<PathBuf> {
        if let Some(d) = &self.state_dir {
            return Ok(d.clone());
        }
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home dir"))?;
        Ok(home.join(".claude-history-sync"))
    }
}
