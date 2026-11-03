//! Hub configuration: database URL, bind address, and the bearer-token →
//! machine-id map. Loaded from a TOML file (path in `HUB_CONFIG`) or, as a
//! convenience for single-machine/dev use, from environment variables.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use uuid::Uuid;

/// The shortest plaintext token a config may hold (openspec
/// hub-read-needs-credential D4, security R4): guessing must stay infeasible
/// even without a per-source limit behind Tailscale serve.
pub const MIN_TOKEN_LEN: usize = 32;

/// Lowercase hex SHA256, the form `[[client_readers]]` stores.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    Sha256::digest(s.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[derive(Debug, Clone, Deserialize)]
pub struct HubConfig {
    pub database_url: String,
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
    #[serde(default)]
    pub tokens: Vec<TokenEntry>,
    /// Read-only tokens (the agents blog), in their own table so that a hub
    /// binary that predates them ignores the table and grants nothing.
    #[serde(default)]
    pub readers: Vec<ReaderEntry>,
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
    /// Credential-shape detection at ingest (Gitea #34). Every rule always
    /// DETECTS and records a finding; only the rules listed here also REPLACE
    /// the value before it is stored. Empty (the default) = flag-only.
    #[serde(default)]
    pub redaction: RedactionConfig,
    /// Client scope (openspec `hub-client-read-scope`). Absent = off: no session is
    /// classified and every reader reads as today.
    #[serde(default)]
    pub client_scope: Option<ClientScopeConfig>,
    /// Client-read principals (design D5, R2-H): read-only, and they read client
    /// sessions. The config holds only the sha256 of each token, never the token.
    #[serde(default)]
    pub client_readers: Vec<ClientReaderEntry>,
    /// The hub's own origins, `scheme://host[:port]` as browsers address it
    /// (on m4m the two tailscale serve fronts). The Tailscale identity header
    /// counts only on a request to one of these names from one of these pages
    /// or from no page (security fe3fd22 F1). Empty: the header never counts.
    #[serde(default)]
    pub public_origins: Vec<String>,
    /// Other pages allowed to call the hub cross-origin (with a bearer token;
    /// never with credentials). Empty, the default: no CORS at all, which the
    /// same-origin webapp does not need.
    #[serde(default)]
    pub cors_origins: Vec<String>,
    /// The key file behind browser-session fingerprints (`/v1/login`). Unset:
    /// `hub-login.key` next to `HUB_CONFIG`, created on first start. Without
    /// either, sessions end at every restart.
    #[serde(default)]
    pub login_key_file: Option<PathBuf>,
}

impl HubConfig {
    /// Where the login key lives, if anywhere (see `login_key_file`).
    pub fn login_key_path(&self) -> Option<PathBuf> {
        self.login_key_file.clone().or_else(|| {
            std::env::var_os("HUB_CONFIG")
                .map(PathBuf::from)
                .and_then(|p| p.parent().map(|d| d.join("hub-login.key")))
        })
    }
}

/// One `[[client_readers]]` entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientReaderEntry {
    /// Lowercase hex sha256 of the bearer token.
    pub token_sha256: String,
    pub machine_id: Uuid,
    pub label: String,
}

/// `[client_scope]` in `hub.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientScopeConfig {
    /// The distiller's routing file (`[protected].path_markers`), outside the repo.
    pub routing_file: PathBuf,
    /// Its floor record (sha256 and minimum counts per marker kind).
    pub floor_file: PathBuf,
    /// Hide client sessions from lab readers. `false` = observe: classify and
    /// count, hide nothing. Once the hub has enforced, `false` no longer turns
    /// enforcement off (the latch, design R2-I).
    #[serde(default)]
    pub enforce: bool,
    /// The only git sources `accept-routing` takes instruction blocks from
    /// (security review V1): the dotfiles repo and the path of ac's
    /// instructions file in it. Empty: no block can be accepted.
    #[serde(default)]
    pub instruction_sources: Vec<InstructionSourceConfig>,
}

/// One allowed source of instruction blocks: a git repository and a path in it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionSourceConfig {
    pub git_repo: PathBuf,
    pub git_path: String,
}

/// `[redaction]` in `hub.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RedactionConfig {
    /// Rule ids in redact mode: any of `pem`, `prefix`, `bearer`, `assign`.
    #[serde(default)]
    pub redact: Vec<String>,
}

impl RedactionConfig {
    /// The redact-mode rules, or an error naming an unknown id: a typo here
    /// must not silently leave a rule in flag mode.
    pub fn rules(&self) -> anyhow::Result<Vec<crate::redact::Rule>> {
        self.redact
            .iter()
            .map(|id| {
                crate::redact::Rule::from_id(id)
                    .ok_or_else(|| anyhow::anyhow!("[redaction] unknown rule id {id:?}"))
            })
            .collect()
    }
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
    /// How long opening the mirror waits while ANOTHER process holds its lock
    /// (a swap's outgoing hub, a stray second hub) before giving up WITHOUT
    /// touching the file (#105). A held file is not unreadable: moving it
    /// aside made the live hub adopt an empty one and cold-build.
    #[serde(default = "default_lock_wait_secs")]
    pub lock_wait_secs: u64,
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
            lock_wait_secs: default_lock_wait_secs(),
        }
    }
}

fn default_lock_wait_secs() -> u64 {
    30
}

fn default_refresh_secs() -> u64 {
    300
}

/// Generous relative to any plausible in-flight transaction, and cheap: the
/// re-scanned rows are already mirrored, so they are ignored on insert.
fn default_overlap_rows() -> i64 {
    50_000
}

/// Still a cap, not tuning. 1 GB was too tight once the mirror reached ~6 GB
/// on disk: on 2026-09-23 a routine incremental COMMIT failed with "failed to
/// pin block (953.5 MiB/953.6 MiB used)", which is what wedged the mirror
/// (cchv #44). 4 GB is 1/16 of m4m's RAM.
fn default_memory_limit() -> String {
    "4GB".to_string()
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

/// `deny_unknown_fields`: a field this binary does not know (`read_only`
/// from revision 1, or a typo) must not load as a full-scope token.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenEntry {
    pub token: String,
    pub machine_id: Uuid,
    #[serde(default)]
    pub label: Option<String>,
    /// Limit this token to `/v1/ingest`: no read routes, no journal writes.
    /// For fenced guests that archive their own sessions (Macs leave it unset).
    #[serde(default)]
    pub ingest_only: bool,
}

/// A read-only token: every authenticated GET answers as for a full token,
/// every other method answers 401 (see `auth`). For agents that read the
/// archive and must not write it (the agents blog). A table of its own, not a
/// flag on `TokenEntry`: a hub binary that predates readers ignores the table
/// and grants the token nothing, where an unknown flag would have been ignored
/// and left a FULL token (security C2).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderEntry {
    pub token: String,
    pub machine_id: Uuid,
    pub label: String,
    /// Limit this reader to the stats-read routes (`/v1/stats/daily`,
    /// `/v1/stats/tool-calls`): every other request is 403, `/v1/login`
    /// included (openspec stats-read-api D3 condition 3). For services that
    /// store counts and must read no message (monitoring, quota-tap). A binary
    /// that predates the flag refuses the whole config (`deny_unknown_fields`),
    /// so it can never load as an unlimited reader.
    #[serde(default)]
    pub stats_only: bool,
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
            cfg.validate()
                .map_err(|e| anyhow::anyhow!("HUB_CONFIG {path}: {e}"))?;
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
                ingest_only: false,
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
            readers: Vec::new(),
            static_dir,
            trust_tailscale_identity,
            embed_model_dir,
            stats_mirror: MirrorConfig {
                path: std::env::var("HUB_STATS_MIRROR").ok().map(PathBuf::from),
                ..MirrorConfig::default()
            },
            // Env-configured hubs are flag-only; redact mode needs `hub.toml`.
            redaction: RedactionConfig::default(),
            // Client scope needs `hub.toml`.
            client_scope: None,
            client_readers: Vec::new(),
            public_origins: Vec::new(),
            cors_origins: Vec::new(),
            login_key_file: None,
        })
    }

    /// The tokens limited to `/v1/ingest` (see `TokenEntry::ingest_only`).
    pub fn ingest_only_tokens(&self) -> HashSet<String> {
        self.tokens
            .iter()
            .filter(|t| t.ingest_only)
            .map(|t| t.token.clone())
            .collect()
    }

    /// The tokens limited to reading (see `TokenEntry::read_only`).
    pub fn read_only_tokens(&self) -> HashSet<String> {
        self.readers.iter().map(|r| r.token.clone()).collect()
    }

    /// The readers limited to the stats-read routes (`ReaderEntry::stats_only`).
    pub fn stats_only_tokens(&self) -> HashSet<String> {
        self.readers
            .iter()
            .filter(|r| r.stats_only)
            .map(|r| r.token.clone())
            .collect()
    }

    /// Refuse a token string or a machine id that appears twice across
    /// `[[tokens]]` and `[[readers]]` (security C1). A token string twice
    /// would carry whichever scopes its entries add up to — a reader leaf
    /// copied from a guest leaf kept the guest's ingest — and a machine id
    /// twice makes the audit principal ambiguous. The error names entries by
    /// label (else machine id), never by token.
    pub fn validate(&self) -> anyhow::Result<()> {
        for (key, list) in [
            ("public_origins", &self.public_origins),
            ("cors_origins", &self.cors_origins),
        ] {
            for o in list {
                if crate::origin_guard::normalize_origin(o).is_none() {
                    anyhow::bail!(
                        "{key}: `{o}` is not an http(s) origin (scheme://host[:port], no path)"
                    );
                }
            }
        }
        if !self.trust_tailscale_identity.is_empty() && self.public_origins.is_empty() {
            tracing::warn!(
                "trust_tailscale_identity is set but public_origins is empty: \
                 the identity header will never count (security fe3fd22 F1)"
            );
        }
        let entries = self
            .tokens
            .iter()
            .map(|t| {
                let who = t
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("machine_id {}", t.machine_id));
                (t.token.as_str(), t.machine_id, format!("token `{who}`"))
            })
            .chain(self.readers.iter().map(|r| {
                (
                    r.token.as_str(),
                    r.machine_id,
                    format!("reader `{}`", r.label),
                )
            }))
            .chain(self.client_readers.iter().map(|r| {
                (
                    r.token_sha256.as_str(),
                    r.machine_id,
                    format!("client reader `{}`", r.label),
                )
            }));
        for r in &self.client_readers {
            if r.token_sha256.len() != 64
                || !r
                    .token_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                anyhow::bail!(
                    "client reader `{}`: token_sha256 must be 64 lowercase hex characters",
                    r.label
                );
            }
        }
        let mut by_token: HashMap<&str, String> = HashMap::new();
        let mut by_machine: HashMap<Uuid, String> = HashMap::new();
        for (token, machine_id, who) in entries {
            if let Some(first) = by_token.insert(token, who.clone()) {
                anyhow::bail!("duplicate token string: {first} and {who} share one");
            }
            if let Some(first) = by_machine.insert(machine_id, who.clone()) {
                anyhow::bail!("duplicate machine_id {machine_id}: {first} and {who}");
            }
        }
        // Fingerprints across tables (openspec hub-read-needs-credential D7,
        // security R3): a client-reader hash equal to the SHA256 of a
        // plaintext token would give that token client reads, so a
        // restricted token could widen. The plain map above compares strings
        // only and cannot see it.
        let client_hashes: std::collections::HashSet<&str> = self
            .client_readers
            .iter()
            .map(|r| r.token_sha256.as_str())
            .collect();
        let plaintext = self
            .tokens
            .iter()
            .map(|t| {
                (
                    t.token.as_str(),
                    format!(
                        "token `{}`",
                        t.label.clone().unwrap_or_else(|| t.machine_id.to_string())
                    ),
                )
            })
            .chain(
                self.readers
                    .iter()
                    .map(|r| (r.token.as_str(), format!("reader `{}`", r.label))),
            );
        for (token, who) in plaintext {
            if token.len() < MIN_TOKEN_LEN {
                anyhow::bail!("{who}: tokens must be at least {MIN_TOKEN_LEN} characters");
            }
            if client_hashes.contains(sha256_hex(token).as_str()) {
                anyhow::bail!(
                    "{who}: a client reader's token_sha256 is the hash of this token; \
                     one token cannot hold two scopes"
                );
            }
        }
        Ok(())
    }

    /// Client-read token hashes → machine id.
    pub fn client_reader_map(&self) -> HashMap<String, Uuid> {
        self.client_readers
            .iter()
            .map(|r| (r.token_sha256.clone(), r.machine_id))
            .collect()
    }

    /// The tokens that may log in at `/v1/login` (security eff460e1 (6)): lab
    /// readers that are not `stats_only`, and client readers. Never a machine
    /// or ingest token.
    pub fn login_eligible(&self) -> Vec<crate::login::Eligible> {
        use crate::visibility::ReadScope;
        self.readers
            .iter()
            .filter(|r| !r.stats_only)
            .map(|r| crate::login::Eligible {
                token_sha256: crate::login::token_sha256(&r.token),
                principal: format!("machine:{}", r.machine_id),
                scope: ReadScope::Lab,
            })
            .chain(self.client_readers.iter().map(|c| crate::login::Eligible {
                token_sha256: c.token_sha256.to_ascii_lowercase(),
                principal: format!("client-reader:{}", c.machine_id),
                scope: ReadScope::Client,
            }))
            .collect()
    }

    /// Build the token → machine-id lookup used by the auth layer.
    pub fn token_map(&self) -> HashMap<String, Uuid> {
        self.tokens
            .iter()
            .map(|t| (t.token.clone(), t.machine_id))
            .chain(self.readers.iter().map(|r| (r.token.clone(), r.machine_id)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "7ac7e806-c4df-4a25-9151-b25ee1a9d636";

    #[test]
    fn token_entries_default_to_full_scope() {
        let entry: TokenEntry =
            toml::from_str(&format!("token = \"t\"\nmachine_id = \"{ID}\"")).unwrap();
        assert!(!entry.ingest_only);
    }

    const ID2: &str = "2b1b6f0e-3d5c-4f39-8a0b-6c1e9b0a7d42";

    fn parse(toml_text: &str) -> anyhow::Result<HubConfig> {
        Ok(toml::from_str(&format!(
            "database_url = \"postgres://x\"\n{toml_text}"
        ))?)
    }

    /// Tokens long enough for `validate` (`MIN_TOKEN_LEN`).
    const MAC: &str = "mac-token-0123456789abcdef0123456789";
    const BLOG: &str = "blog-token-0123456789abcdef012345678";

    #[test]
    fn readers_are_collected_into_the_token_map_and_the_reader_set() {
        let config = parse(&format!(
            "[[tokens]]\ntoken = \"{MAC}\"\nmachine_id = \"{ID}\"\n\
             [[readers]]\ntoken = \"{BLOG}\"\nmachine_id = \"{ID2}\"\nlabel = \"agents-blog\"\n"
        ))
        .unwrap();
        assert_eq!(config.token_map().len(), 2);
        assert_eq!(config.read_only_tokens(), HashSet::from([BLOG.to_string()]));
        assert!(config.ingest_only_tokens().is_empty());
        config.validate().unwrap();
    }

    /// C2: under `[[tokens]]` a `read_only` flag must not load. An older
    /// binary would ignore it and serve a FULL token, so the format must
    /// never be written; this binary refuses it outright.
    #[test]
    fn read_only_under_tokens_does_not_load() {
        let err = parse(&format!(
            "[[tokens]]\ntoken = \"t\"\nmachine_id = \"{ID}\"\nread_only = true\n"
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("read_only"), "{err}");
    }

    #[test]
    fn an_unknown_reader_field_does_not_load() {
        assert!(parse(&format!(
            "[[readers]]\ntoken = \"t\"\nmachine_id = \"{ID}\"\nlabel = \"r\"\ningest_only = true\n"
        ))
        .is_err());
    }

    /// hub-read-needs-credential D7 (security R3): a client-reader hash equal
    /// to the SHA256 of an ingest or reader token is refused, without printing
    /// the token; a different hash is fine.
    #[test]
    fn a_client_hash_of_a_plaintext_token_is_refused() {
        let token = "t".repeat(40);
        let hash = super::sha256_hex(&token);
        for table in ["tokens", "readers"] {
            let extra = if table == "tokens" {
                "ingest_only = true\n"
            } else {
                ""
            };
            let cfg = format!(
                "[[{table}]]\ntoken = \"{token}\"\nmachine_id = \"{ID}\"\nlabel = \"guest\"\n{extra}\
                 [[client_readers]]\ntoken_sha256 = \"{hash}\"\nmachine_id = \"{ID2}\"\nlabel = \"distiller\"\n"
            );
            let err = parse(&cfg).unwrap().validate().unwrap_err().to_string();
            assert!(err.contains("two scopes"), "{table}: {err}");
            assert!(
                !err.contains(&token),
                "{table}: the error must not print a token: {err}"
            );
        }
        let other = super::sha256_hex(&"u".repeat(40));
        let ok = format!(
            "[[tokens]]\ntoken = \"{token}\"\nmachine_id = \"{ID}\"\nlabel = \"m4m\"\n\
             [[client_readers]]\ntoken_sha256 = \"{other}\"\nmachine_id = \"{ID2}\"\nlabel = \"distiller\"\n"
        );
        parse(&ok).unwrap().validate().unwrap();
    }

    /// hub-read-needs-credential D4 (security R4): a plaintext token under 32
    /// characters is refused at load.
    #[test]
    fn a_short_token_is_refused() {
        for (table, len, refused) in [
            ("tokens", 31, true),
            ("readers", 31, true),
            ("tokens", 32, false),
        ] {
            let cfg = format!(
                "[[{table}]]\ntoken = \"{}\"\nmachine_id = \"{ID}\"\nlabel = \"x\"\n",
                "k".repeat(len)
            );
            let r = parse(&cfg).unwrap().validate();
            assert_eq!(r.is_err(), refused, "{table} {len}: {r:?}");
        }
    }

    /// C1: a reader leaf copied from a guest leaf (same token) or any repeat
    /// of a token string or machine id across both tables is refused, and
    /// the error never prints the token.
    #[test]
    fn duplicates_across_tables_fail_validation() {
        let cases = [
            // same token, different machine ids (guest leaf copied to reader)
            format!(
                "[[tokens]]\ntoken = \"secret-value\"\nmachine_id = \"{ID}\"\nlabel = \"guest\"\ningest_only = true\n\
                 [[readers]]\ntoken = \"secret-value\"\nmachine_id = \"{ID2}\"\nlabel = \"blog\"\n"
            ),
            // same machine id, different tokens
            format!(
                "[[tokens]]\ntoken = \"secret-a\"\nmachine_id = \"{ID}\"\nlabel = \"m4m\"\n\
                 [[readers]]\ntoken = \"secret-b\"\nmachine_id = \"{ID}\"\nlabel = \"blog\"\n"
            ),
            // two Mac tokens sharing a string
            format!(
                "[[tokens]]\ntoken = \"secret-value\"\nmachine_id = \"{ID}\"\nlabel = \"m4m\"\n\
                 [[tokens]]\ntoken = \"secret-value\"\nmachine_id = \"{ID2}\"\nlabel = \"mbp\"\n"
            ),
        ];
        for case in cases {
            let err = parse(&case).unwrap().validate().unwrap_err().to_string();
            assert!(err.contains("duplicate"), "{err}");
            assert!(
                !err.contains("secret-"),
                "the error must not print a token: {err}"
            );
        }
    }

    #[test]
    fn ingest_only_tokens_are_collected() {
        let config: HubConfig = toml::from_str(&format!(
            "database_url = \"postgres://x\"\n\
             [[tokens]]\ntoken = \"mac\"\nmachine_id = \"{ID}\"\n\
             [[tokens]]\ntoken = \"guest\"\nmachine_id = \"{ID}\"\ningest_only = true\n"
        ))
        .unwrap();
        assert_eq!(config.token_map().len(), 2);
        assert_eq!(
            config.ingest_only_tokens(),
            HashSet::from(["guest".to_string()])
        );
    }
}
