//! Shared application state: the Postgres pool, the token → machine-id map,
//! and the (optional) sentence embedder for semantic journal search.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::embed::Embedder;
use crate::mirror::Mirror;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Bearer token → machine id. `Arc` so the state stays cheap to clone per request.
    pub tokens: Arc<HashMap<String, Uuid>>,
    /// Tokens (a subset of `tokens`) limited to `/v1/ingest`: refused by every
    /// read route and every other machine-token write. For fenced guests that
    /// archive their own sessions but must not read the archive.
    pub ingest_only: Arc<HashSet<String>>,
    /// Tokens (a subset of `tokens`) limited to reading: accepted by
    /// `Authenticated`, refused by every write (see `auth`).
    pub read_only: Arc<HashSet<String>>,
    /// Tailscale logins granted READ scope via the `Tailscale-User-Login`
    /// header (see `HubConfig::trust_tailscale_identity`). Empty = disabled.
    pub trusted_identities: Arc<Vec<String>>,
    /// Sentence embedder for semantic journal search. `None` (unconfigured)
    /// and a configured-but-failed load both degrade queries to keyword.
    pub embedder: Option<Arc<dyn Embedder>>,
    /// Wakes the embedding sweep early (journal writes nudge it so fresh
    /// entries become semantically searchable without waiting an interval).
    /// Always present; notifying with no sweep listening is a no-op.
    pub embed_nudge: Arc<Notify>,
    /// The statistics read model. `None` means the mirror could not be opened
    /// at all (a disk-level fault) — every other subsystem keeps working and
    /// `/v1/stats/*` answers 503, which is the same answer a warming mirror
    /// gives, so there is one degraded shape rather than two.
    pub mirror: Option<Arc<Mirror>>,
    /// Credential rules in redact mode (`[redaction] redact`). Empty means
    /// every rule only flags (Gitea #34).
    pub redact_rules: Arc<Vec<crate::redact::Rule>>,
    /// Client scope (`[client_scope]`). `None` = off: nothing is classified
    /// and every reader reads as before.
    pub client_scope: Option<Arc<crate::client_state::ClientScope>>,
    /// sha256(token) → machine id of the client-read principals (R2-H).
    pub client_readers: Arc<HashMap<String, Uuid>>,
    /// Browser sessions minted by `/v1/login` (R2-H).
    pub logins: crate::login::Logins,
}

impl AppState {
    pub fn new(
        pool: PgPool,
        tokens: HashMap<String, Uuid>,
        trusted_identities: Vec<String>,
    ) -> Self {
        Self {
            pool,
            tokens: Arc::new(tokens),
            ingest_only: Arc::new(HashSet::new()),
            read_only: Arc::new(HashSet::new()),
            trusted_identities: Arc::new(trusted_identities),
            embedder: None,
            embed_nudge: Arc::new(Notify::new()),
            mirror: None,
            redact_rules: Arc::new(Vec::new()),
            client_scope: None,
            client_readers: Arc::new(HashMap::new()),
            logins: crate::login::Logins::default(),
        }
    }

    /// The state `run()` serves from, minus the embedder and the mirror: the
    /// token map, the trusted identities, the ingest-only and reader sets and
    /// the redaction rules, all from one `HubConfig`. Tests build their state
    /// here too, so a scope wired in the config but dropped on the way to the
    /// state fails a test (security C3).
    pub fn from_config(pool: PgPool, config: &crate::config::HubConfig) -> anyhow::Result<Self> {
        Ok(Self::new(
            pool,
            config.token_map(),
            config.trust_tailscale_identity.clone(),
        )
        .with_ingest_only_tokens(config.ingest_only_tokens())
        .with_read_only_tokens(config.read_only_tokens())
        .with_redact_rules(config.redaction.rules()?)
        .with_client_scope(config.client_scope.clone())
        .with_client_readers(config.client_reader_map()))
    }

    /// Client-read principals by token hash (builder-style, same reason).
    pub fn with_client_readers(mut self, readers: HashMap<String, Uuid>) -> Self {
        self.client_readers = Arc::new(readers);
        self
    }

    /// Security K1: the latch lives in the database, so a config without
    /// `[client_scope]` (a stale pre-feature render, a deleted section) must not
    /// lift it. When the latch is set and the section is missing, client scope is
    /// turned on with no policy: lab readers see no session, and
    /// `/v1/healthz/client-scope` answers 503.
    pub async fn with_orphan_latch(mut self) -> anyhow::Result<Self> {
        if self.client_scope.is_some() {
            return Ok(self);
        }
        let latched: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar("SELECT enforced_at FROM client_scope_state")
                .fetch_optional(&self.pool)
                .await?
                .flatten();
        if latched.is_some() {
            tracing::error!(
                "client scope was enforced, but the config has no [client_scope]: \
                 every session is client to lab readers until the section is restored"
            );
            self.client_scope = Some(Arc::new(crate::client_state::ClientScope::without_section()));
        }
        Ok(self)
    }

    /// Turn client scope on with this config (builder-style, same reason).
    /// Call [`crate::client_state::ClientScope::refresh`] before serving.
    pub fn with_client_scope(mut self, cfg: Option<crate::config::ClientScopeConfig>) -> Self {
        self.client_scope = cfg.map(|c| Arc::new(crate::client_state::ClientScope::new(c)));
        self
    }

    /// Limit these tokens to `/v1/ingest` (builder-style, same reason).
    pub fn with_ingest_only_tokens(mut self, tokens: HashSet<String>) -> Self {
        self.ingest_only = Arc::new(tokens);
        self
    }

    /// Limit these tokens to reading (builder-style, same reason).
    pub fn with_read_only_tokens(mut self, tokens: HashSet<String>) -> Self {
        self.read_only = Arc::new(tokens);
        self
    }

    /// Put these credential rules in redact mode (builder-style, same reason).
    pub fn with_redact_rules(mut self, rules: Vec<crate::redact::Rule>) -> Self {
        self.redact_rules = Arc::new(rules);
        self
    }

    /// Attach an embedder (builder-style so existing constructions stay valid).
    pub fn with_embedder(mut self, embedder: Arc<dyn Embedder>) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Attach the statistics mirror (builder-style, same reason).
    pub fn with_mirror(mut self, mirror: Arc<Mirror>) -> Self {
        self.mirror = Some(mirror);
        self
    }
}
