//! Hub configuration: database URL, bind address, and the bearer-token →
//! machine-id map. Loaded from a TOML file (path in `HUB_CONFIG`) or, as a
//! convenience for single-machine/dev use, from environment variables.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct HubConfig {
    pub database_url: String,
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
    #[serde(default)]
    pub tokens: Vec<TokenEntry>,
    /// When set, serve this directory's files at `/` (static archive webapp).
    /// `/v1/*` routes always win; unset keeps the plain-404 fallback.
    #[serde(default)]
    pub static_dir: Option<PathBuf>,
    /// Tailscale logins granted READ scope when the request carries a
    /// matching `Tailscale-User-Login` header (injected by Tailscale serve
    /// for tailnet clients; Funnel traffic gets none). Opt-in — empty means
    /// bearer-only. Ingest always requires a bearer token.
    #[serde(default)]
    pub trust_tailscale_identity: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenEntry {
    pub token: String,
    pub machine_id: Uuid,
    #[serde(default)]
    pub label: Option<String>,
}

fn default_bind_addr() -> String {
    "0.0.0.0:8787".to_string()
}

impl HubConfig {
    /// Load from the TOML file at `HUB_CONFIG`, else from environment variables
    /// (`DATABASE_URL`, `HUB_BIND_ADDR`, optional `HUB_STATIC_DIR`, optional
    /// comma-separated `HUB_TRUST_TAILSCALE_IDENTITY`, and optional
    /// single-machine `HUB_TOKEN` + `HUB_MACHINE_ID`).
    pub fn load() -> anyhow::Result<Self> {
        if let Ok(path) = std::env::var("HUB_CONFIG") {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("reading HUB_CONFIG {path}: {e}"))?;
            let cfg: HubConfig = toml::from_str(&text)
                .map_err(|e| anyhow::anyhow!("parsing HUB_CONFIG {path}: {e}"))?;
            return Ok(cfg);
        }

        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL or HUB_CONFIG must be set"))?;
        let bind_addr = std::env::var("HUB_BIND_ADDR").unwrap_or_else(|_| default_bind_addr());
        let mut tokens = Vec::new();
        if let (Ok(token), Ok(machine_id)) =
            (std::env::var("HUB_TOKEN"), std::env::var("HUB_MACHINE_ID"))
        {
            tokens.push(TokenEntry {
                token,
                machine_id: machine_id.parse()?,
                label: None,
            });
        }
        let static_dir = std::env::var("HUB_STATIC_DIR").ok().map(PathBuf::from);
        let trust_tailscale_identity = std::env::var("HUB_TRUST_TAILSCALE_IDENTITY")
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Ok(HubConfig {
            database_url,
            bind_addr,
            tokens,
            static_dir,
            trust_tailscale_identity,
        })
    }

    /// Build the token → machine-id lookup used by the auth layer.
    pub fn token_map(&self) -> HashMap<String, Uuid> {
        self.tokens
            .iter()
            .map(|t| (t.token.clone(), t.machine_id))
            .collect()
    }
}
