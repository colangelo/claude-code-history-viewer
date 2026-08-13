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
    /// Directory holding the sentence-embedding model (config.json +
    /// tokenizer.json + model.safetensors) for semantic journal search.
    /// Unset, missing, or corrupt → semantic modes degrade to keyword;
    /// never a startup failure.
    #[serde(default)]
    pub embed_model_dir: Option<PathBuf>,
    /// Statistics read model. Every field is optional with a working default,
    /// so an existing `hub.toml` keeps running untouched.
    #[serde(default)]
    pub stats_mirror: MirrorConfig,
}

/// Settings for the `DuckDB` statistics mirror (change `hub-stats-duckdb-mirror`).
#[derive(Debug, Clone, Deserialize)]
pub struct MirrorConfig {
    /// Where the mirror file lives. Defaults to `stats-mirror.duckdb` beside
    /// the hub's other state under `~/.config/cchv`.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// How often the background refresher pulls new rows from Postgres.
    #[serde(default = "default_refresh_secs")]
    pub refresh_secs: u64,
    /// How far *behind* the high watermark each refresh re-scans. Rows are
    /// keyed at insert but commit out of order, so a refresh that trusted the
    /// watermark alone would skip late-committing rows permanently (design D2).
    /// Inserts are idempotent, which is what makes the overlap free.
    #[serde(default = "default_overlap_rows")]
    pub overlap_rows: i64,
    /// `DuckDB` `memory_limit`. m4m also runs the daemon and the distiller, so
    /// statistics must not be able to consume the box.
    #[serde(default = "default_memory_limit")]
    pub memory_limit: String,
    /// `DuckDB` `threads`, capped for the same reason.
    #[serde(default = "default_threads")]
    pub threads: u32,
    /// Age past which `/v1/healthz/stats` reports the mirror stale.
    #[serde(default = "default_stale_after_secs")]
    pub stale_after_secs: u64,
    /// Wall-clock ceiling on one *incremental* refresh attempt.
    ///
    /// Exists because "the refresh failed" and "the refresh never returned" are
    /// different faults and only the first one degrades gracefully. A Postgres
    /// socket that dies without an `RST` — a tailnet path flap relaying the
    /// peer via DERP is enough — leaves the client blocked in `read()` with no
    /// error to report, forever: `acquire_timeout` does not apply once the
    /// connection is out of the pool, and no statement timeout is set.
    #[serde(default = "default_refresh_timeout_secs")]
    pub refresh_timeout_secs: u64,
    /// The same ceiling for a *cold build*, which legitimately takes minutes
    /// (the whole archive, not an increment) and must not be cancelled by the
    /// incremental budget — a cold build killed on every tick would never
    /// finish and `/v1/stats/*` would answer 503 permanently.
    #[serde(default = "default_cold_build_timeout_secs")]
    pub cold_build_timeout_secs: u64,
}

impl Default for MirrorConfig {
    fn default() -> Self {
        Self {
            path: None,
            refresh_secs: default_refresh_secs(),
            overlap_rows: default_overlap_rows(),
            memory_limit: default_memory_limit(),
            threads: default_threads(),
            stale_after_secs: default_stale_after_secs(),
            refresh_timeout_secs: default_refresh_timeout_secs(),
            cold_build_timeout_secs: default_cold_build_timeout_secs(),
        }
    }
}

fn default_refresh_secs() -> u64 {
    300
}

/// Generous relative to any plausible in-flight transaction, and cheap: the
/// re-scanned rows are already mirrored, so they are ignored on insert.
fn default_overlap_rows() -> i64 {
    50_000
}

fn default_memory_limit() -> String {
    "1GB".to_string()
}

fn default_threads() -> u32 {
    2
}

fn default_stale_after_secs() -> u64 {
    3_600
}

/// Two orders of magnitude above a healthy incremental refresh (seconds), and
/// still inside [`default_stale_after_secs`] so a wedged tick is cancelled and
/// retried *before* the mirror is old enough to page anyone. Sized to survive a
/// large legitimate catch-up over a DERP-relayed link, not to be a tight bound.
fn default_refresh_timeout_secs() -> u64 {
    900
}

/// A cold build of the real archive is millions of rows; the ceiling is a
/// backstop against a wedge, not a performance budget, so it is deliberately
/// far above the observed build time.
fn default_cold_build_timeout_secs() -> u64 {
    21_600
}

impl MirrorConfig {
    /// Resolved mirror path: the configured one, else `~/.config/cchv/`, else
    /// the current directory if even `HOME` is unset.
    pub fn resolved_path(&self) -> PathBuf {
        self.path.clone().unwrap_or_else(|| {
            std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".config/cchv/stats-mirror.duckdb")
        })
    }
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
    /// `HUB_EMBED_MODEL_DIR`, optional comma-separated
    /// `HUB_TRUST_TAILSCALE_IDENTITY`, and optional single-machine
    /// `HUB_TOKEN` + `HUB_MACHINE_ID`).
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
        let embed_model_dir = std::env::var("HUB_EMBED_MODEL_DIR").ok().map(PathBuf::from);
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
            embed_model_dir,
            stats_mirror: MirrorConfig {
                path: std::env::var("HUB_STATS_MIRROR").ok().map(PathBuf::from),
                ..MirrorConfig::default()
            },
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
